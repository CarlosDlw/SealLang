//! Monomorphization and vtable collection for protocol calls.

use seal_def::DefId;
use seal_index::Idx;
use seal_mir::{CallTarget, Rvalue, StatementKind};

use crate::tests::support::{Fixture, collect_clean, names};
use crate::{MonoMir, MonoProgram, MonoVTable, VtableId, instance_name};

const GRID: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
}";

fn instance_named<'a>(
    fixture: &Fixture,
    hir: &seal_hir::Hir,
    program: &'a MonoProgram,
    name: &str,
) -> &'a MonoMir {
    program
        .instances()
        .iter()
        .find(|instance| instance_name(hir, fixture.interner(), instance.instance) == name)
        .unwrap_or_else(|| {
            panic!(
                "no instance named `{name}`, have {:?}",
                names(fixture, hir, program)
            )
        })
}

/// Returns every call target in `body`, in statement order.
fn call_targets(body: &seal_mir::Body) -> Vec<CallTarget> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind;
            match rvalue {
                Rvalue::Call { target, .. } => Some(*target),
                _ => None,
            }
        })
        .collect()
}

fn direct_defs(body: &seal_mir::Body) -> Vec<DefId> {
    call_targets(body)
        .into_iter()
        .map(|target| match target {
            CallTarget::Direct { instance, .. } => instance.def,
            other => panic!("expected a direct call, got {other:?}"),
        })
        .collect()
}

#[test]
fn concrete_protocol_calls_become_direct_calls() {
    let source = format!(
        "{GRID} func main() -> i32 {{ \
           let mut grid = Grid {{ value: 40 }}; \
           grid[0] += 1; \
           grid[0] \
         }}"
    );
    let (fixture, hir, _mir, program) = collect_clean(&source);
    program.validate(&hir).expect("the program is valid");
    let main = instance_named(&fixture, &hir, &program, "main");
    let defs = direct_defs(&main.body);
    let called: Vec<String> = defs
        .iter()
        .map(|def| fixture.symbol(hir.definitions().name(*def)))
        .collect();
    assert!(
        called.contains(&"index".to_owned()),
        "the read/compound path calls Index::index: {called:?}"
    );
    assert!(
        called.contains(&"index_mut".to_owned()),
        "the write path calls IndexMut::index_mut: {called:?}"
    );
}

#[test]
fn a_generic_index_function_is_instantiated_twice() {
    let source = "\
        struct First { value: i32 } \
        struct Second { value: i32 } \
        impl Index<usize, i32> for First { \
            func index(self: *First, index: usize) -> *i32 { &(*self).value } \
        } \
        impl Index<usize, i32> for Second { \
            func index(self: *Second, index: usize) -> *i32 { &(*self).value } \
        } \
        func read<G: Index<usize, i32>>(grid: *G) -> i32 { grid[0] } \
        func main() -> i32 { \
            let first = First { value: 1 }; \
            let second = Second { value: 2 }; \
            read(&first) + read(&second) \
        }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    program.validate(&hir).expect("the program is valid");
    let first = instance_named(&fixture, &hir, &program, "read<First>");
    let second = instance_named(&fixture, &hir, &program, "read<Second>");
    let first_defs = direct_defs(&first.body);
    let second_defs = direct_defs(&second.body);
    assert_eq!(first_defs.len(), 1, "one index call");
    assert_eq!(second_defs.len(), 1, "one index call");
    assert_ne!(
        first_defs[0], second_defs[0],
        "each instantiation selects its own impl"
    );
    let selected = fixture.symbol(hir.definitions().name(first_defs[0]));
    assert_eq!(selected, "index");
}

#[test]
fn a_generic_iterator_loop_is_instantiated_twice() {
    let source = "\
        struct First { value: i32 } \
        struct Second { value: i32 } \
        impl Iterator<i32> for First { \
            func has_next(self: *First) -> bool { (*self).value < 1 } \
            func next(self: *mut First) -> i32 { (*self).value } \
        } \
        impl Iterator<i32> for Second { \
            func has_next(self: *Second) -> bool { (*self).value < 1 } \
            func next(self: *mut Second) -> i32 { (*self).value } \
        } \
        func consume<I: Iterator<i32>>(iter: I) -> i32 { \
            let mut total = 0; \
            for value in iter { total += value; } \
            total \
        } \
        func main() -> i32 { \
            consume(First { value: 1 }) + consume(Second { value: 2 }) \
        }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    program.validate(&hir).expect("the program is valid");
    let first = instance_named(&fixture, &hir, &program, "consume<First>");
    let second = instance_named(&fixture, &hir, &program, "consume<Second>");
    let first_defs = direct_defs(&first.body);
    let second_defs = direct_defs(&second.body);
    assert_eq!(first_defs.len(), 2, "has_next and next");
    assert_eq!(second_defs.len(), 2, "has_next and next");
    assert_ne!(
        first_defs, second_defs,
        "each instantiation selects its own Iterator impl"
    );
}

#[test]
fn nested_custom_indexing_collects_both_implementations() {
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
            let outer = Outer { inner: Inner { value: 42 } }; \
            outer[0][0] \
        }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    program.validate(&hir).expect("the program is valid");
    let rendered = names(&fixture, &hir, &program);
    assert_eq!(
        rendered
            .iter()
            .filter(|name| name.as_str() == "index")
            .count(),
        2,
        "both Index impls are collected: {rendered:?}"
    );
    let main = instance_named(&fixture, &hir, &program, "main");
    let defs = direct_defs(&main.body);
    assert_eq!(defs.len(), 2, "nested indexing emits two protocol calls");
    assert_ne!(defs[0], defs[1], "the calls target distinct impls");
}

#[test]
fn dyn_index_and_index_mut_collect_vtables_and_dynamic_calls() {
    let source = format!(
        "{GRID} \
         func read(grid: *dyn Index<usize, i32>) -> i32 {{ grid[0] }} \
         func write(grid: *mut dyn IndexMut<usize, i32>) {{ grid[0] = 1; }} \
         func main() {{ \
           let mut grid = Grid {{ value: 0 }}; \
           let shared: *dyn Index<usize, i32> = &grid; \
           let read_value = read(shared); \
           let exclusive: *mut dyn IndexMut<usize, i32> = &mut grid; \
           write(exclusive); \
           let _ = read_value; \
         }}"
    );
    let (fixture, hir, _mir, program) = collect_clean(&source);
    program.validate(&hir).expect("the program is valid");
    assert_eq!(program.vtables().len(), 2, "one vtable per protocol");
    let index = program.vtable(VtableId::from_usize(0));
    let index_mut = program.vtable(VtableId::from_usize(1));
    let table_names = |table: &MonoVTable| {
        table
            .entries
            .iter()
            .map(|entry| instance_name(&hir, fixture.interner(), *entry))
            .collect::<Vec<_>>()
    };
    let tables = [table_names(index), table_names(index_mut)];
    assert!(
        tables.contains(&vec!["index".to_owned()]),
        "the shared vtable has the index slot: {tables:?}"
    );
    assert!(
        tables.contains(&vec!["index_mut".to_owned()]),
        "the exclusive vtable has the index_mut slot: {tables:?}"
    );

    let dynamic_calls = program
        .instances()
        .iter()
        .flat_map(|instance| instance.body.blocks.iter())
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::DynamicCall { .. },
                    ..
                }
            )
        })
        .count();
    assert_eq!(dynamic_calls, 2, "read and write dispatch dynamically");
}

#[test]
fn dyn_into_iterator_is_rejected_before_monomorphization() {
    let source = "\
        struct Source { value: i32 } \
        func take(value: *dyn IntoIterator<Source, i32>) {} \
        func main() { \
            let source = Source { value: 0 }; \
            let dynamic: *dyn IntoIterator<Source, i32> = &source; \
            take(dynamic); \
        }";
    let fixture = Fixture::new(source);
    let hir = fixture.hir_with_errors();
    fixture.expect("E2065", "cannot be used as a trait object");
    assert!(
        !hir.dyn_types().is_empty(),
        "the trait-object use was recorded and rejected by object safety"
    );
}
