//! Lowering of custom `Index` and `IndexMut` protocol calls.
//!
//! The protocol call is recorded by type checking. Lowering must reuse the
//! existing call and borrow rvalues, evaluate the index operand once, and read
//! or write through the pointer returned by the protocol.

use seal_ty::RefKind;

use crate::body::Body;
use crate::operand::{Constant, Operand};
use crate::place::{Place, Projection};
use crate::rvalue::{CallTarget, Rvalue};

use super::support::{assignment, function, lower_clean};

const GRID_MUT: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
}";

fn assignments(body: &Body) -> Vec<(&Place, &Rvalue)> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(assignment)
        .collect()
}

fn call_name(
    fixture: &super::support::Fixture,
    hir: &seal_hir::Hir,
    rvalue: &Rvalue,
) -> Option<String> {
    let Rvalue::Call { target, .. } = rvalue else {
        return None;
    };
    let name = hir.definitions().name(target.instance().def);
    Some(fixture.symbol(name))
}

fn is_call(
    fixture: &super::support::Fixture,
    hir: &seal_hir::Hir,
    rvalue: &Rvalue,
    name: &str,
) -> bool {
    call_name(fixture, hir, rvalue).as_deref() == Some(name)
}

fn is_deref_of(place: &Place, local: crate::id::LocalId) -> bool {
    place.local == local && place.projections() == [Projection::Deref]
}

#[test]
fn a_custom_index_write_calls_index_mut_and_writes_through_its_pointer() {
    let source =
        format!("{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; grid[0] = 2; }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let (call_place, call) = values
        .iter()
        .find(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .copied()
        .expect("the write calls index_mut");
    let Rvalue::Call { target, args, .. } = call else {
        unreachable!("call_name only matches calls");
    };
    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(args.len(), 2, "receiver and index");
    assert!(
        matches!(args[0], Operand::Copy(_)),
        "the receiver is borrowed"
    );
    assert!(matches!(
        args[1],
        Operand::Constant(Constant::Int { value: 0, .. })
    ));

    let deref = Place::with_projections(call_place.local, vec![Projection::Deref]);
    assert!(
        values.iter().any(|(place, rvalue)| {
            *place == &deref
                && matches!(
                    rvalue,
                    Rvalue::Use(Operand::Constant(Constant::Int { value: 2, .. }))
                )
        }),
        "the assignment writes through the returned pointer"
    );
    assert!(
        values.iter().all(|(place, _)| {
            !place
                .projections()
                .iter()
                .any(|projection| matches!(projection, Projection::Index { .. }))
        }),
        "custom indexing must not lower to a built-in index projection"
    );
}

#[test]
fn a_custom_index_read_dereferences_the_index_pointer() {
    let source =
        format!("{GRID_MUT} func main() -> i32 {{ let grid = Grid {{ value: 7 }}; grid[0] }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    assert!(
        values
            .iter()
            .any(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index")),
        "the read calls Index::index"
    );
    let (target, rvalue) = values
        .iter()
        .find(|(place, _)| place.local == Body::return_local())
        .expect("the return place is written");
    assert!(target.is_whole());
    let Rvalue::Use(Operand::Copy(place)) = rvalue else {
        panic!("the read copies the indexed place");
    };
    assert!(matches!(place.projections(), [Projection::Deref]));
}

#[test]
fn the_index_operand_runs_once_before_the_receiver_borrow() {
    let source = format!(
        "{GRID_MUT} \
         func key() -> usize {{ 0 }} \
         func main() {{ let mut grid = Grid {{ value: 1 }}; grid[key()] = 2; }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let key = values
        .iter()
        .position(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "key"))
        .expect("the index operand calls key");
    let receiver = values
        .iter()
        .position(|(_, rvalue)| {
            matches!(
                rvalue,
                Rvalue::Ref {
                    kind: RefKind::Exclusive,
                    ..
                }
            )
        })
        .expect("the receiver is borrowed exclusively");
    let index = values
        .iter()
        .position(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .expect("the write calls index_mut");
    assert!(
        key < receiver && receiver < index,
        "expected key < receiver borrow < index_mut, got {key}, {receiver}, {index}"
    );
    assert_eq!(
        values
            .iter()
            .filter(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "key"))
            .count(),
        1,
        "the index operand must be evaluated exactly once"
    );
}

#[test]
fn a_simple_assignment_evaluates_the_value_before_the_target_place() {
    let source = format!(
        "{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; grid[0] = grid[0] + 1; }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let shared = values
        .iter()
        .position(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index"))
        .expect("the value reads through Index");
    let exclusive = values
        .iter()
        .position(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .expect("the target writes through IndexMut");
    assert!(
        shared < exclusive,
        "the value read must end before the target borrow, got {shared} and {exclusive}"
    );
}

#[test]
fn a_compound_assignment_uses_the_indexed_place_for_bothsides() {
    let source =
        format!("{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; grid[0] += 2; }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let (call_place, _) = values
        .iter()
        .find(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .copied()
        .expect("the compound assignment calls index_mut once");
    let binary = values
        .iter()
        .find_map(|(place, rvalue)| match rvalue {
            Rvalue::Binary { lhs, .. } if place.local == call_place.local => Some(lhs),
            _ => None,
        })
        .expect("the old value is read from the indexed place");
    assert_eq!(
        binary,
        &Operand::Copy(Place::with_projections(
            call_place.local,
            vec![Projection::Deref]
        ))
    );
    assert_eq!(
        values
            .iter()
            .filter(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
            .count(),
        1
    );
}

#[test]
fn nested_custom_indices_lower_each_protocol_call_once() {
    let source = "\
        struct Inner { value: i32 } \
        struct Outer { inner: Inner } \
        impl Index<usize, Inner> for Outer { \
            func index(self: *Outer, index: usize) -> *Inner { &(*self).inner } \
        } \
        impl Index<usize, i32> for Inner { \
            func index(self: *Inner, index: usize) -> *i32 { &(*self).value } \
        } \
        func main() -> i32 { \
            let outer = Outer { inner: Inner { value: 7 } }; \
            outer[0][0] \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    assert_eq!(
        values
            .iter()
            .filter(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index"))
            .count(),
        2,
        "one Index call per level"
    );
    let (target, rvalue) = values
        .iter()
        .find(|(place, _)| place.local == Body::return_local())
        .expect("the return place is written");
    assert!(target.is_whole());
    let Rvalue::Use(Operand::Copy(place)) = rvalue else {
        panic!("the nested read copies the final place");
    };
    assert!(matches!(place.projections(), [Projection::Deref]));
}

#[test]
fn a_field_after_a_custom_index_keeps_the_deref_and_field_projection() {
    let source = "\
        struct Item { value: i32 } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        func main() -> i32 { let grid = Grid { item: Item { value: 7 } }; grid[0].value }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    assert!(
        values
            .iter()
            .any(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index"))
    );
    let (target, rvalue) = values
        .iter()
        .find(|(place, _)| place.local == Body::return_local())
        .expect("the return place is written");
    assert!(target.is_whole());
    let Rvalue::Use(Operand::Copy(place)) = rvalue else {
        panic!("the field read copies the projected place");
    };
    assert!(matches!(
        place.projections(),
        [Projection::Deref, Projection::Field(_)]
    ));
}

#[test]
fn a_method_after_a_custom_index_borrows_the_dereferenced_element() {
    let source = "\
        struct Item { value: i32 } \
        impl Item { func read(self: *Item) -> i32 { (*self).value } } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        func main() -> i32 { let grid = Grid { item: Item { value: 7 } }; grid[0].read() }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let names: Vec<String> = values
        .iter()
        .filter_map(|(_, rvalue)| call_name(&fixture, &hir, rvalue))
        .collect();
    assert_eq!(names, vec!["index".to_owned(), "read".to_owned()]);
    let (_, read) = values
        .iter()
        .find(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "read"))
        .expect("the method call is lowered");
    let Rvalue::Call { args, .. } = read else {
        unreachable!("call_name only matches calls");
    };
    let Operand::Copy(reference) = &args[0] else {
        panic!("the receiver is a borrowed temporary");
    };
    assert_eq!(
        body.local(reference.local).kind,
        crate::body::LocalKind::Temp
    );
}

#[test]
fn an_exclusive_borrow_of_an_indexed_element_uses_index_mut() {
    let source = format!(
        "{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; let value = &mut grid[0]; *value = 2; }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let (call_place, _) = values
        .iter()
        .find(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .copied()
        .expect("the exclusive borrow uses index_mut");
    assert!(
        values.iter().any(|(_, rvalue)| matches!(
            rvalue,
            Rvalue::Ref {
                place,
                kind: RefKind::Exclusive,
            } if is_deref_of(place, call_place.local)
        )),
        "the borrow covers the dereferenced element"
    );
}

#[test]
fn a_dyn_index_mut_receiver_lowers_to_a_dynamic_call() {
    let source = "func write(grid: *mut dyn IndexMut<usize, i32>) { grid[0] = 1; }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "write");
    let values = assignments(body);
    let (call_place, dynamic) = values
        .iter()
        .find_map(|(place, rvalue)| match rvalue {
            Rvalue::DynamicCall { method, args, .. } => Some((*place, (method, args))),
            _ => None,
        })
        .expect("the write dispatches through the vtable");
    let (method, args) = dynamic;
    assert_eq!(fixture.symbol(hir.definitions().name(*method)), "index_mut");
    assert_eq!(
        args.len(),
        1,
        "the index argument; the receiver is separate"
    );
    assert!(
        values.iter().any(|(place, rvalue)| {
            matches!(
                rvalue,
                Rvalue::Use(Operand::Constant(Constant::Int { value: 1, .. }))
            ) && is_deref_of(place, call_place.local)
        }),
        "the assignment writes through the dynamic call result"
    );
}

#[test]
fn an_outer_pointer_index_mut_borrows_the_pointer_binding() {
    let source = "\
        struct Grid { value: i32 } \
        impl IndexMut<usize, i32> for *mut Grid { \
            func index_mut(self: *mut *mut Grid, index: usize) -> *mut i32 { &mut (**self).value } \
        } \
        func main() { \
            let mut grid = Grid { value: 1 }; \
            let mut pointer: *mut Grid = &mut grid; \
            pointer[0] = 2; \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let values = assignments(body);
    let (call_place, _) = values
        .iter()
        .find(|(_, rvalue)| is_call(&fixture, &hir, rvalue, "index_mut"))
        .copied()
        .expect("the outer pointer impl is selected");
    assert!(
        values.iter().any(|(place, rvalue)| matches!(
            rvalue,
            Rvalue::Ref {
                kind: RefKind::Exclusive,
                ..
            } if !place.is_deref()
        )),
        "the pointer binding itself is borrowed exclusively"
    );
    assert!(values.iter().any(|(place, rvalue)| matches!(
        rvalue,
        Rvalue::Use(Operand::Constant(Constant::Int { value: 2, .. }))
    ) && is_deref_of(place, call_place.local)));
}
