//! Type checking for constants, statics, and constant contexts.

use seal_def::{DefId, DefKind};
use seal_hir::{ExprKind, Hir};

use crate::{PlaceRoot, Signature};

use super::support::{check_clean, check_errors, function, tail, ty_text_with};

fn definition(fixture: &super::support::Fixture, hir: &Hir, kind: DefKind, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

fn signature(results: &crate::TypeckResults, def: DefId) -> &Signature {
    results.signature(def).expect("the item has a signature")
}

const POINT: &str = "struct Point { x: i32, y: i32 }";
const SHAPE: &str = "struct Circle { radius: i32 } \
                     trait Shape { func area(self: *Self) -> i32; } \
                     impl Shape for Circle { \
                         func area(self: *Circle) -> i32 { (*self).radius } \
                     }";

#[test]
fn value_items_receive_parameterless_signatures() {
    let source = "const ANSWER: i32 = 42; static LIMIT: i32 = 3;";
    let (fixture, hir, results) = check_clean(source);

    let answer = definition(&fixture, &hir, DefKind::Const, "ANSWER");
    let limit = definition(&fixture, &hir, DefKind::Static, "LIMIT");
    for (def, name) in [(answer, "i32"), (limit, "i32")] {
        let signature = signature(&results, def);
        assert!(signature.params.is_empty(), "a value has no parameters");
        assert_eq!(
            hir.ty_ctxt().kind(signature.ret),
            hir.ty_ctxt().kind(hir.body_of(def).unwrap().ret.unwrap()),
            "the signature uses the declared {name} type"
        );
    }
}

#[test]
fn const_and_static_paths_resolve_in_value_and_place_modes() {
    let source = format!(
        "{POINT} const ORIGIN: Point = Point {{ x: 0, y: 0 }}; \
         static LIMIT: i32 = 3; \
         func main() -> i32 {{ ORIGIN.x }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let origin = definition(&fixture, &hir, DefKind::Const, "ORIGIN");
    let (_, body) = function(&fixture, &hir, "main");
    let field = tail(&hir, body);

    let place = results.place(field).expect("a field access is a place");
    assert_eq!(place.root, PlaceRoot::Const(origin));
    assert_eq!(place.projections.len(), 1);
}

#[test]
fn borrowing_and_indexing_a_const_are_places() {
    let source = "const VALUES: [i32; 2] = [1, 2]; \
                  func main() -> i32 { let values: *[i32; 2] = &VALUES; VALUES[0] }";
    let (fixture, hir, results) = check_clean(source);
    let values = definition(&fixture, &hir, DefKind::Const, "VALUES");
    let (_, body) = function(&fixture, &hir, "main");
    let index = tail(&hir, body);
    assert_eq!(
        results.place(index).expect("an index is a place").root,
        PlaceRoot::Const(values)
    );
}

#[test]
fn reading_a_non_copy_static_by_value_is_rejected() {
    let source = format!(
        "{POINT} static HOME: Point = Point {{ x: 0, y: 0 }}; \
         func main() -> Point {{ HOME }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3058", "cannot move `HOME` out of a static");
}

#[test]
fn reading_a_copy_field_of_a_static_is_allowed() {
    let source = format!(
        "{POINT} static HOME: Point = Point {{ x: 4, y: 2 }}; \
         func main() -> i32 {{ HOME.x }}"
    );
    let (fixture, hir, _) = check_clean(&source);
    let home = definition(&fixture, &hir, DefKind::Static, "HOME");
    assert_eq!(
        hir.definitions().kind(home),
        DefKind::Static,
        "the field read still names the static"
    );
}

#[test]
fn a_const_non_copy_value_is_produced_fresh() {
    let source = format!(
        "{POINT} const HOME: Point = Point {{ x: 0, y: 0 }}; \
         func main() -> Point {{ HOME }}"
    );
    let _ = check_clean(&source);
}

#[test]
fn assigning_to_an_immutable_static_is_rejected() {
    let source = "static LIMIT: i32 = 3; func main() { LIMIT = 4; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3059", "immutable static `LIMIT`");
}

#[test]
fn an_exclusive_borrow_of_an_immutable_static_is_rejected() {
    let source = "static LIMIT: i32 = 3; func main() -> *mut i32 { &mut LIMIT }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3017", "immutable item `LIMIT`");
}

#[test]
fn assigning_to_a_const_is_rejected() {
    let source = "const LIMIT: i32 = 3; func main() { LIMIT = 4; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3005", "constant `LIMIT`");
}

#[test]
fn a_mutable_static_requires_an_unsafe_block() {
    let source = "static mut COUNTER: i32 = 0; func main() -> i32 { COUNTER }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3063", "requires an `unsafe` block");
}

#[test]
fn a_mutable_static_write_requires_an_unsafe_block() {
    let source = "static mut COUNTER: i32 = 0; func main() { COUNTER = 1; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3063", "requires an `unsafe` block");
}

#[test]
fn an_unsafe_block_permits_mutable_static_access() {
    let source = "static mut COUNTER: i32 = 0; \
                  func main() -> i32 { unsafe { COUNTER = COUNTER + 1; COUNTER } }";
    let (fixture, hir, _) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    assert!(matches!(
        hir.expr(tail(&hir, body)).kind,
        ExprKind::Unsafe { .. }
    ));
}

#[test]
fn an_unsafe_block_has_the_type_of_its_block() {
    let source = "func main() -> i32 { unsafe { 42 } }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let ty = results
        .expr_ty(tail(&hir, body))
        .expect("the unsafe block is typed");
    assert_eq!(ty_text_with(&fixture, &hir, ty), "i32");
}

#[test]
fn an_unsized_static_is_rejected_with_its_own_code() {
    let (fixture, _, _) = check_errors("static TEXT: str = \"hi\";");
    fixture.expect("E3056", "static must have a `Sized` type");
}

#[test]
fn an_unsized_const_is_rejected_with_its_own_code() {
    let (fixture, _, _) = check_errors("const TEXT: str = \"hi\";");
    fixture.expect("E3057", "constant must have a `Sized` type");
}

#[test]
fn a_const_fn_cannot_use_a_mutable_reference() {
    let source = "const func count(values: *mut i32) -> i32 { 0 }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3062", "cannot use `*mut T`");
}

#[test]
fn a_const_value_cannot_contain_a_mutable_reference() {
    let source = "static mut VALUE: i32 = 0; static POINTER: *mut i32 = &mut VALUE;";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4012", "cannot contain a mutable reference");
}

#[test]
fn a_constant_initializer_cannot_call_a_non_const_function() {
    let source = "func build() -> i32 { 1 } const VALUE: i32 = build();";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4011", "cannot call a non-`const` function");
}

#[test]
fn a_const_fn_cannot_call_a_non_const_function() {
    let source = "func build() -> i32 { 1 } const func value() -> i32 { build() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4018", "cannot call a non-`const` function");
}

#[test]
fn a_constant_initializer_cannot_read_a_mutable_static() {
    let source = "static mut COUNTER: i32 = 0; const VALUE: i32 = unsafe { COUNTER };";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4026", "cannot read a mutable static");
}

#[test]
fn a_const_fn_cannot_access_a_mutable_static() {
    let source = "static mut COUNTER: i32 = 0; \
                  const func value() -> i32 { unsafe { COUNTER } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4020", "cannot access a mutable static");
}

#[test]
fn a_constant_initializer_cannot_use_a_custom_index_protocol() {
    let source = "struct Grid { value: i32 } \
                  impl Index<usize, i32> for Grid { \
                      func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
                  } \
                  static GRID: Grid = Grid { value: 3 }; \
                  const VALUE: i32 = GRID[0];";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4017", "cannot use a custom protocol");
}

#[test]
fn a_const_fn_cannot_dispatch_dynamically() {
    let source = format!("{SHAPE} const func measure(shape: *dyn Shape) -> i32 {{ shape.area() }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E4021", "dynamic or trait-bound dispatch");
}

#[test]
fn a_constant_initializer_cannot_dispatch_dynamically() {
    let source = format!(
        "{SHAPE} static SHAPE: *dyn Shape = &Circle {{ radius: 2 }}; \
         const AREA: i32 = SHAPE.area();"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E4016", "cannot use dynamic dispatch");
}

#[test]
fn a_const_fn_cannot_call_through_a_trait_bound() {
    let source = "trait Shape { func area(self: *Self) -> i32; } \
                  const func area<T: Shape>(value: T) -> i32 { value.area() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E4021", "trait-bound dispatch");
}
