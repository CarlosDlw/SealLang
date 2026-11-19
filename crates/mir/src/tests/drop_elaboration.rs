//! Drop elaboration: guards, replacement drops, and dead sites.
//!
//! `impl Drop` is not part of the surface yet, so a type needs a destructor in
//! one of two ways. A type parameter answers `true` because monomorphization
//! decides the concrete type, and an aggregate can be registered through the
//! destructor registry before MIR lowering, which is what the surface will do
//! once it exists. The tests use both.

use seal_def::DefKind;
use seal_hir::Hir;

use crate::body::{Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::mir::Mir;
use crate::operand::{Constant, Operand};
use crate::place::{Place, Projection};
use crate::rvalue::Rvalue;
use crate::statement::{DropKind, StatementKind};
use crate::terminator::TerminatorKind;

use super::support::{Fixture, function, local_named, lower_clean};

/// Lowers `source` and elaborates its drop sites.
fn elaborate(source: &str) -> (Fixture, Hir, Mir) {
    let (fixture, hir, mut mir) = lower_clean(source);
    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration succeeds");
    mir.validate_with(&hir)
        .expect("elaborated MIR keeps its invariants");
    (fixture, hir, mir)
}

/// Lowers `source` with the named aggregates implementing `Drop`, then
/// elaborates its drop sites.
fn elaborate_with_destructors(source: &str, names: &[&str]) -> (Fixture, Hir, Mir) {
    let fixture = Fixture::new(source);
    let (hir, mut mir) = fixture.lower_with_destructors(names);
    fixture.assert_clean();
    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration succeeds");
    mir.validate_with(&hir)
        .expect("elaborated MIR keeps its invariants");
    (fixture, hir, mir)
}

/// Returns the drop flags of `body`, in declaration order.
fn flag_locals(body: &Body) -> Vec<LocalId> {
    body.locals
        .iter_enumerated()
        .filter(|(_, declaration)| declaration.kind == LocalKind::DropFlag)
        .map(|(local, _)| local)
        .collect()
}

/// Returns every drop statement of `body` with its kind and place.
fn drops(body: &Body) -> Vec<(DropKind, Place)> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            StatementKind::Drop { place, kind } => Some((*kind, place.clone())),
            _ => None,
        })
        .collect()
}

/// Returns the drop flag whose guard drops `dropped`.
///
/// Only a compiler flag counts: a user `bool` also appears in a `SwitchInt`, and
/// the block it selects may drop the place for an unrelated reason.
fn flag_guarding(body: &Body, dropped: LocalId) -> Option<LocalId> {
    body.blocks.iter().find_map(|block| {
        let TerminatorKind::SwitchInt {
            discriminant: Operand::Copy(place),
            targets,
            ..
        } = &block.terminator.kind
        else {
            return None;
        };
        let flag = place.local_id()?;
        if body.local(flag).kind != LocalKind::DropFlag {
            return None;
        }
        let target = targets.iter().find(|target| target.value == 1)?.target;
        let guard_drops = body.block(target).statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                StatementKind::Drop { place, .. }
                    if place.local_id() == Some(dropped)
            )
        });
        guard_drops.then_some(flag)
    })
}

/// Returns the flags `body` sets to `true` somewhere, in block order.
fn flags_set_true(body: &Body) -> Vec<LocalId> {
    let mut flags: Vec<LocalId> = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Use(Operand::Constant(Constant::Bool(true))),
            } => place.local_id(),
            _ => None,
        })
        .filter(|local| body.local(*local).kind == LocalKind::DropFlag)
        .collect();
    flags.dedup();
    flags
}

/// Returns the value `body` assigns to `flag` at entry.
fn entry_value_of(body: &Body, flag: LocalId) -> Option<bool> {
    body.block(body.entry)
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                place,
                rvalue: Rvalue::Use(Operand::Constant(Constant::Bool(value))),
            } if place.local_id() == Some(flag) => Some(*value),
            _ => None,
        })
}

/// Returns every assignment of `body` with its block, position, and
/// destination.
fn destinations(body: &Body) -> Vec<(BasicBlockId, usize, &Place)> {
    body.blocks
        .iter_enumerated()
        .flat_map(|(block, data)| {
            data.statements
                .iter()
                .enumerate()
                .filter_map(move |(position, statement)| match &statement.kind {
                    StatementKind::Assign { place, .. } => Some((block, position, place)),
                    _ => None,
                })
        })
        .collect()
}

/// Returns the field a place selects when it is one field deep, if any.
fn field_name(hir: &Hir, fixture: &Fixture, place: &Place) -> Option<String> {
    match place.projections.as_slice() {
        [Projection::Field(field)] => Some(fixture.symbol(hir.definitions().get(*field).name)),
        _ => None,
    }
}

#[test]
fn a_definitely_initialized_binding_is_dropped_without_a_flag() {
    let (fixture, hir, mir) = elaborate("func f<T>(value: T) { let inner = value; }\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert!(
        flag_locals(body).is_empty(),
        "a value that is always there needs no flag"
    );
    let dropped: Vec<LocalId> = drops(body)
        .iter()
        .filter_map(|(_, place)| place.local_id())
        .collect();
    assert_eq!(dropped, [local_named(&fixture, body, "inner")]);
}

#[test]
fn a_binding_assigned_on_one_path_is_guarded_by_a_flag() {
    let (fixture, hir, mir) = elaborate(
        "func f<T>(value: T, flag: bool) {\n  let mut picked: T;\n  if flag { picked = value; }\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let picked = local_named(&fixture, body, "picked");

    let flag = flag_guarding(body, picked).expect("the conditional binding is guarded");
    assert_eq!(body.local(flag).kind, LocalKind::DropFlag);
    assert_eq!(
        entry_value_of(body, flag),
        Some(false),
        "the binding starts without a value"
    );
}

#[test]
fn a_binding_that_is_never_assigned_has_no_drop() {
    let (fixture, hir, mir) = elaborate("func f<T>() { let unused: T; }\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert!(drops(body).is_empty(), "there is no value to drop");
    assert!(flag_locals(body).is_empty(), "a dead site needs no flag");
}

#[test]
fn a_loop_that_reinitializes_a_binding_keeps_one_drop() {
    let (fixture, hir, mir) = elaborate(
        "func f<T: Copy>(value: T, flag: bool) {\n  let mut current = value;\n  while flag { current = value; }\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let current = local_named(&fixture, body, "current");
    assert!(
        flag_guarding(body, current).is_none(),
        "the binding holds a value at its site on every path"
    );
    assert!(flag_locals(body).is_empty());
}

#[test]
fn assigning_on_every_branch_keeps_the_site_unconditional() {
    let (fixture, hir, mir) = elaborate(
        "func f<T>(first: T, second: T, flag: bool) {\n  let mut picked: T;\n  if flag { picked = first; } else { picked = second; }\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let picked = local_named(&fixture, body, "picked");
    assert!(
        flag_guarding(body, picked).is_none(),
        "both branches initialize the binding, so the site is unconditional"
    );
}

#[test]
fn a_move_clears_the_flag_of_a_parameter() {
    let (fixture, hir, mir) =
        elaborate("func f<T>(value: T, flag: bool) {\n  if flag { let taken = value; }\n}\n");
    let body = function(&fixture, &hir, &mir, "f");
    let value = local_named(&fixture, body, "value");
    let flag = flag_guarding(body, value).expect("the moved parameter is guarded");
    assert_eq!(
        entry_value_of(body, flag),
        Some(true),
        "a parameter starts with a value, and the branch takes it away"
    );
}

#[test]
fn assigning_over_an_initialized_value_drops_the_old_one() {
    let (fixture, hir, mir) = elaborate(
        "func f<T>(first: T, second: T) {\n  let mut current = first;\n  current = second;\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let current = local_named(&fixture, body, "current");
    let (block, position, _) = destinations(body)
        .into_iter()
        .rev()
        .find(|(_, _, place)| place.local_id() == Some(current))
        .expect("the binding is assigned");
    let previous = &body.block(block).statements[position - 1];
    assert!(
        matches!(
            &previous.kind,
            StatementKind::Drop {
                place,
                kind: DropKind::Overwrite,
            } if place.local_id() == Some(current)
        ),
        "the replaced value is dropped first, got {:?}",
        previous.kind
    );
}

#[test]
fn assigning_to_an_uninitialized_value_inserts_no_drop() {
    let (fixture, hir, mir) =
        elaborate("func f<T>(value: T) {\n  let current: T;\n  current = value;\n}\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert!(
        drops(body).iter().all(|(kind, _)| *kind == DropKind::Scope),
        "an uninitialized destination has no old value to drop"
    );
}

#[test]
fn assigning_a_field_drops_the_old_field() {
    let (fixture, hir, mir) = elaborate(
        "struct Wrapper<A, B> { first: A, second: B }\n\n\
         func f<T>(first: T, second: T, third: T) {\n  \
         let mut wrapper = Wrapper { first: first, second: second };\n  \
         wrapper.first = third;\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let wrapper = local_named(&fixture, body, "wrapper");
    let (block, position, _) = destinations(body)
        .into_iter()
        .find(|(_, _, place)| field_name(&hir, &fixture, place).as_deref() == Some("first"))
        .expect("the field is assigned");
    let previous = &body.block(block).statements[position - 1];
    let StatementKind::Drop {
        place,
        kind: DropKind::Overwrite,
    } = &previous.kind
    else {
        panic!("expected a replacement drop, got {:?}", previous.kind);
    };
    assert_eq!(place.local_id(), Some(wrapper));
    assert_eq!(
        field_name(&hir, &fixture, place).as_deref(),
        Some("first"),
        "only the replaced field is dropped"
    );
}

#[test]
fn assigning_a_whole_struct_sets_the_flags_of_its_fields() {
    let (fixture, hir, mir) = elaborate(
        "struct Wrapper<A, B> { first: A, second: B }\n\n\
         func f<T: Copy>(first: T, second: T, flag: bool) {\n  \
         let mut wrapper: Wrapper<T, T>;\n  \
         if flag { wrapper = Wrapper { first: first, second: second }; }\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let flags = flag_locals(body);
    assert_eq!(flags.len(), 2, "one flag per field's drop site");
    assert_eq!(
        flags_set_true(body),
        flags,
        "writing the whole value initializes both fields"
    );
}

#[test]
fn assigning_through_a_reference_drops_the_old_field() {
    let (fixture, hir, mir) = elaborate(
        "struct Wrapper<A> { first: A }\n\n\
         func f<T>(pointer: *mut Wrapper<T>, value: T) { (*pointer).first = value; }\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let replaced = drops(body)
        .into_iter()
        .find(|(kind, _)| *kind == DropKind::Overwrite)
        .expect("the field the assignment replaces is dropped");
    assert_eq!(replaced.1.projections.len(), 2);
    assert!(matches!(
        replaced.1.projections.as_slice(),
        [Projection::Deref, Projection::Field(_)]
    ));
}

#[test]
fn assigning_a_static_inserts_no_drop() {
    let (fixture, hir, mir) = elaborate_with_destructors(
        "struct Pair { first: i32, second: i32 }\n\n\
         func destroy(value: *mut Pair) {}\n\n\
         static mut VALUE: Pair = Pair { first: 1, second: 2 };\n\n\
         func main() { unsafe { VALUE = Pair { first: 3, second: 4 }; } }\n",
        &["Pair"],
    );
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        drops(body)
            .iter()
            .all(|(kind, _)| *kind != DropKind::Overwrite),
        "a static lives for the whole program and is never dropped"
    );
}

#[test]
fn a_conditional_site_in_a_constant_is_guarded() {
    let (fixture, hir, mir) = elaborate_with_destructors(
        "struct Pair { first: i32, second: i32 }\n\n\
         func destroy(value: *mut Pair) {}\n\n\
         const IF: bool = true;\n\n\
         const VALUE: () = {\n  let mut built: Pair;\n  \
         if IF { built = Pair { first: 1, second: 2 }; }\n};\n",
        &["Pair"],
    );
    let owner = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Const && fixture.symbol(data.name) == "VALUE")
        .map(|(id, _)| id)
        .expect("the source declares the constant");
    let body = mir.body_of(owner).expect("the constant has a body");
    let built = local_named(&fixture, body, "built");
    assert_eq!(flag_locals(body).len(), 1);
    assert!(
        flag_guarding(body, built).is_some(),
        "the constant's binding is dropped only when the branch built it"
    );
}

#[test]
fn a_struct_without_a_destructor_drops_its_fields_in_declaration_order() {
    let (fixture, hir, mir) = elaborate(
        "struct Wrapper<A, B> { first: A, second: B }\n\n\
         func f<T>(first: T, second: T) {\n  \
         let wrapper = Wrapper { first: first, second: second };\n}\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let fields: Vec<Option<String>> = drops(body)
        .iter()
        .map(|(_, place)| field_name(&hir, &fixture, place))
        .collect();
    assert_eq!(
        fields,
        [Some("first".to_owned()), Some("second".to_owned())]
    );
}

#[test]
fn an_enum_is_dropped_as_one_value() {
    let (fixture, hir, mir) = elaborate(
        "enum Either<T> { Left(T), Right(T) }\n\n\
         func f<T>(value: T) { let either = Either::Left(value); }\n",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let either = local_named(&fixture, body, "either");
    let dropped = drops(body);
    assert_eq!(dropped.len(), 1, "the variant glue drops the payload");
    assert!(dropped[0].1.is_whole());
    assert_eq!(dropped[0].1.local_id(), Some(either));
}

#[test]
fn an_array_is_dropped_as_one_value() {
    let (fixture, hir, mir) =
        elaborate("func f<T>(first: T, second: T) { let values = [first, second]; }\n");
    let body = function(&fixture, &hir, &mir, "f");
    let values = local_named(&fixture, body, "values");
    let dropped = drops(body);
    assert_eq!(dropped.len(), 1, "the array glue walks the elements");
    assert!(dropped[0].1.is_whole());
    assert_eq!(dropped[0].1.local_id(), Some(values));
}

#[test]
fn a_type_with_a_destructor_is_dropped_as_one_value() {
    let (fixture, hir, mir) = elaborate_with_destructors(
        "struct Wrapper<A, B> { first: A, second: B }\n\n\
         func destroy(value: *mut Wrapper<i32, i32>) {}\n\n\
         func main() { let wrapper = Wrapper { first: 1, second: 2 }; }\n",
        &["Wrapper"],
    );
    let body = function(&fixture, &hir, &mir, "main");
    let wrapper = local_named(&fixture, body, "wrapper");
    let dropped = drops(body);
    assert_eq!(
        dropped.len(),
        1,
        "the declared destructor answers for the value"
    );
    assert!(dropped[0].1.is_whole());
    assert_eq!(dropped[0].1.local_id(), Some(wrapper));
}

#[test]
fn a_dead_site_in_a_constant_is_removed() {
    let fixture = Fixture::new(
        "struct Pair { first: i32, second: i32 }\n\n\
         func destroy(value: *mut Pair) {}\n\n\
         const VALUE: Pair = {\n  \
         let built = Pair { first: 1, second: 2 };\n  built\n};\n",
    );
    let (hir, mut mir) = fixture.lower_with_destructors(&["Pair"]);
    fixture.assert_clean();
    let owner = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Const)
        .map(|(id, _)| id)
        .expect("the source declares a constant");
    assert!(
        !drops(mir.body_of(owner).expect("the constant has a body")).is_empty(),
        "lowering emits a site for the initializer's binding"
    );

    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration succeeds");
    mir.validate_with(&hir)
        .expect("elaborated MIR keeps its invariants");
    let body = mir.body_of(owner).expect("the constant has a body");
    assert!(
        drops(body).is_empty(),
        "the temporary was moved into the constant's value, so nothing is dropped"
    );
    assert!(flag_locals(body).is_empty());
}

#[test]
fn elaborating_twice_does_not_change_the_body() {
    let (fixture, hir, mut mir) = lower_clean(
        "func f<T>(value: T, flag: bool) {\n  let mut picked: T;\n  if flag { picked = value; }\n}\n",
    );
    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration succeeds");
    let elaborated = function(&fixture, &hir, &mir, "f").clone();
    crate::elaborate_drops(&hir, &mut mir).expect("a repeated elaboration succeeds");
    assert_eq!(
        function(&fixture, &hir, &mir, "f"),
        &elaborated,
        "the second run must leave the body untouched"
    );
}
