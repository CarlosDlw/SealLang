//! Reachable global collection.

use seal_const_eval::AllocSource;
use seal_def::{DefId, DefKind};

use crate::tests::support::{Fixture, collect_clean, collect_clean_constants, collect_errors};
use crate::{MonoItem, MonoProgram, MonoValidationError};

fn static_named(fixture: &Fixture, hir: &seal_hir::Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Static && fixture.symbol(data.name) == name)
        .map(|(def, _)| def)
        .unwrap_or_else(|| panic!("no static named `{name}`"))
}

fn const_named(fixture: &Fixture, hir: &seal_hir::Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Const && fixture.symbol(data.name) == name)
        .map(|(def, _)| def)
        .unwrap_or_else(|| panic!("no const named `{name}`"))
}

fn static_names(fixture: &Fixture, hir: &seal_hir::Hir, program: &MonoProgram) -> Vec<String> {
    program
        .statics()
        .iter()
        .map(|def| fixture.symbol(hir.definitions().name(*def)))
        .collect()
}

#[test]
fn only_reachable_statics_are_collected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "static USED: i32 = 1;\
         static UNUSED: i32 = 2;\
         func main() -> i32 { USED }",
    );
    assert_eq!(static_names(&fixture, &hir, &program), ["USED"]);
}

#[test]
fn statics_referenced_by_other_statics_are_collected_transitively() {
    let (fixture, hir, _mir, program) = collect_clean(
        "static BASE: i32 = 1;\
         static REF: *i32 = &BASE;\
         func main() -> i32 { *REF }",
    );
    assert_eq!(static_names(&fixture, &hir, &program), ["REF", "BASE"]);
}

#[test]
fn a_local_static_is_one_item_per_declaration() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func value<T>(input: T) -> i32 { static COUNT: i32 = 1; COUNT }\
         func main() -> i32 { value(1) + value(true) }",
    );
    assert_eq!(static_names(&fixture, &hir, &program), ["COUNT"]);
    assert_eq!(program.statics().len(), 1);
}

#[test]
fn a_default_body_local_static_is_one_item_across_impls() {
    let (fixture, hir, _mir, program) = collect_clean(
        "struct First {}\
         struct Second {}\
         trait Count { func value(self: *Self) -> i32 { static COUNT: i32 = 1; COUNT } }\
         impl Count for First {}\
         impl Count for Second {}\
         func main() -> i32 { First {}.value() + Second {}.value() }",
    );
    assert_eq!(static_names(&fixture, &hir, &program), ["COUNT"]);
}

#[test]
fn a_const_allocation_referenced_by_code_is_collected() {
    let (fixture, hir, _mir, constants, program) = collect_clean_constants(
        "const VALUE: i32 = 5;\
         const REF: *i32 = &VALUE;\
         func main() -> i32 { *REF }",
    );
    let value = const_named(&fixture, &hir, "VALUE");
    assert_eq!(program.const_allocations().len(), 1);
    let allocation = program.const_allocations()[0];
    let source = constants.allocations()[allocation].source;
    assert!(
        matches!(source, AllocSource::Const(def) if def == value)
            || matches!(source, AllocSource::Promoted { .. }),
        "the allocation belongs to the addressed const or its promotion"
    );
    assert!(
        program
            .items()
            .iter()
            .any(|item| item == &MonoItem::ConstAlloc(allocation))
    );
}

#[test]
fn a_static_trait_object_collects_its_vtable_and_methods() {
    let (fixture, hir, _mir, program) = collect_clean(
        "trait Shape { func area(self: *Self) -> i32; }\
         struct Square { side: i32 }\
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } }\
         static SQUARE: Square = Square { side: 3 };\
         const OBJECT: *dyn Shape = &SQUARE;\
         func main() -> i32 { let object = OBJECT; object.area() }",
    );
    assert_eq!(static_names(&fixture, &hir, &program), ["SQUARE"]);
    assert_eq!(program.vtables().len(), 1);
    // A constant trait object must key its table on the concrete pointee, the
    // same way a runtime coercion does, or the emitted global would name a
    // table the backend cannot resolve.
    program
        .validate(&hir)
        .expect("the constant trait object has a consistent vtable key");
    assert!(
        program
            .vtables()
            .iter()
            .flat_map(|table| &table.entries)
            .any(|entry| fixture.symbol(hir.definitions().name(entry.def)) == "area"),
        "the vtable method instance is collected"
    );
}

#[test]
fn discovery_order_is_deterministic() {
    let source = "static FIRST: i32 = 1;\
                  static SECOND: i32 = 2;\
                  func main() -> i32 { FIRST + SECOND }";
    let (_, _, _, first) = collect_clean(source);
    let (_, _, _, second) = collect_clean(source);
    assert_eq!(first.items(), second.items());
    assert_eq!(first.statics(), second.statics());
    assert_eq!(first.const_allocations(), second.const_allocations());
}

#[test]
fn constant_evaluation_rejects_with_len() {
    let (fixture, _, _, _) = collect_errors(
        "static BYTES: [u8; 4] = [115, 101, 97, 108];\
         const SLICE: *raw [u8] = unsafe { (&BYTES[0] as *raw u8).with_len(4) };",
    );
    fixture.expect("E4032", "cannot read through this raw pointer");
}

#[test]
fn constant_evaluation_rejects_as_str() {
    let (fixture, _, _, _) = collect_errors(
        "static BYTES: [u8; 4] = [115, 101, 97, 108];\
         const SLICE: *raw [u8] = &BYTES[..] as *raw [u8];\
         const TEXT: *raw str = unsafe { SLICE.as_str() };",
    );
    fixture.expect("E4032", "cannot read through this raw pointer");
}

#[test]
fn validation_rejects_a_missing_static() {
    let (fixture, hir, _mir, constants, program) = collect_clean_constants(
        "static VALUE: i32 = 1;\
         func main() -> i32 { VALUE }",
    );
    let value = static_named(&fixture, &hir, "VALUE");
    let mut malformed = program;
    malformed.statics.clear();
    malformed
        .items
        .retain(|item| !matches!(item, MonoItem::Static(_)));
    assert_eq!(
        malformed.validate_globals(&constants),
        Err(MonoValidationError::MissingStatic { def: value })
    );
}

#[test]
fn validation_rejects_a_missing_const_allocation() {
    let (_fixture, _hir, _mir, constants, program) = collect_clean_constants(
        "const VALUE: i32 = 5;\
         const REF: *i32 = &VALUE;\
         func main() -> i32 { *REF }",
    );
    let allocation = program.const_allocations()[0];
    let mut malformed = program;
    malformed.const_allocations.clear();
    malformed
        .items
        .retain(|item| !matches!(item, MonoItem::ConstAlloc(_)));
    assert_eq!(
        malformed.validate_globals(&constants),
        Err(MonoValidationError::MissingConstAllocation { alloc: allocation })
    );
}

#[test]
fn validation_rejects_a_missing_constant_vtable() {
    let (_fixture, _hir, _mir, constants, program) = collect_clean_constants(
        "trait Shape { func area(self: *Self) -> i32; }\
         struct Square { side: i32 }\
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } }\
         static SQUARE: Square = Square { side: 3 };\
         const OBJECT: *dyn Shape = &SQUARE;\
         func main() -> i32 { let object = OBJECT; object.area() }",
    );
    let mut malformed = program;
    malformed.vtables = seal_index::IndexVec::new();
    malformed.vtable_ids.clear();
    let result = malformed.validate_globals(&constants);
    assert!(
        matches!(result, Err(MonoValidationError::MissingConstVtable { .. })),
        "expected a missing constant vtable, got {result:?}"
    );
}

#[test]
fn validation_accepts_collected_globals() {
    let (_fixture, _hir, _mir, constants, program) = collect_clean_constants(
        "const VALUE: i32 = 5;\
         const REF: *i32 = &VALUE;\
         func main() -> i32 { *REF }",
    );
    program
        .validate_globals(&constants)
        .expect("collected globals are complete");
}
