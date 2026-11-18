//! Custom indexing through the `Index` protocol.

use crate::{IndexProtocol, MethodKind, ReceiverAdjust};

use super::support::{check_clean, check_errors, expr_ty, function, tail, ty_text_with};

const GRID: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
}";

const TWO_INDEX_TYPES: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl Index<bool, i32> for Grid { \
    func index(self: *Grid, index: bool) -> *i32 { &(*self).value } \
}";

fn custom(results: &crate::TypeckResults, expr: seal_hir::HirExprId) -> &crate::ProtocolCall {
    match results.index_protocol(expr) {
        Some(IndexProtocol::Custom {
            mutable: false,
            call,
            ..
        }) => call,
        other => panic!("expected custom read indexing, got {other:?}"),
    }
}

#[test]
fn a_custom_index_read_records_the_trait_call() {
    let source =
        format!("{GRID} func main() -> i32 {{ let grid = Grid {{ value: 42 }}; grid[0] }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let index = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, index)),
        "i32"
    );

    let Some(IndexProtocol::Custom {
        mutable: false,
        element_ty,
        call,
    }) = results.index_protocol(index)
    else {
        panic!("the read uses the custom protocol");
    };
    assert_eq!(ty_text_with(&fixture, &hir, *element_ty), "i32");
    assert!(matches!(
        call.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::SharedBorrow);
    assert_eq!(
        ty_text_with(&fixture, &hir, call.method.receiver_ty),
        "Grid"
    );
    assert_eq!(call.method.arg_tys.len(), 1);
    assert_eq!(
        ty_text_with(&fixture, &hir, call.method.arg_tys[0]),
        "usize"
    );
    assert_eq!(ty_text_with(&fixture, &hir, call.method.ret_ty), "*i32");
    assert!(
        call.instance.is_some(),
        "a concrete impl records its instance"
    );
}

#[test]
fn a_generic_bound_supplies_a_symbolic_index_call() {
    let source = format!("{GRID} func read<G: Index<usize, i32>>(grid: *G) -> i32 {{ grid[0] }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "read");
    let index = tail(&hir, body);
    let call = custom(&results, index);
    assert!(matches!(call.method.kind, MethodKind::TraitBound { .. }));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::None);
    assert!(call.instance.is_none(), "a bound has no concrete instance");
    assert_eq!(ty_text_with(&fixture, &hir, call.method.ret_ty), "*i32");
}

#[test]
fn the_index_expression_selects_a_non_usize_key_type() {
    let source = "struct Grid { value: i32 } \
         impl Index<bool, i32> for Grid { \
             func index(self: *Grid, index: bool) -> *i32 { &(*self).value } \
         } \
         func main() -> i32 { let grid = Grid { value: 7 }; grid[true] }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = custom(&results, tail(&hir, body));
    assert_eq!(call.method.arg_tys.len(), 1);
    assert_eq!(ty_text_with(&fixture, &hir, call.method.arg_tys[0]), "bool");
}

#[test]
fn the_index_expression_selects_among_several_implementations() {
    let source = format!(
        "{TWO_INDEX_TYPES} func main() -> i32 {{ \
             let grid = Grid {{ value: 7 }}; \
             grid[true] \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = custom(&results, tail(&hir, body));
    assert_eq!(ty_text_with(&fixture, &hir, call.method.arg_tys[0]), "bool");
}

#[test]
fn an_unselectable_index_expression_is_ambiguous() {
    let source = format!(
        "{TWO_INDEX_TYPES} func main() -> i32 {{ let grid = Grid {{ value: 7 }}; grid[0] }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3053", "cannot choose an `Index` implementation");
}

#[test]
fn a_type_without_an_impl_cannot_be_indexed() {
    let source = "struct Grid { value: i32 } \
                  func main() -> i32 { let grid = Grid { value: 7 }; grid[0] }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3021", "cannot index a value");
}

#[test]
fn a_wrong_index_key_reports_a_mismatch() {
    let source = format!(
        "{GRID} func main() -> i32 {{ \
             let grid = Grid {{ value: 7 }}; \
             let key: i32 = 0; \
             grid[key] \
         }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn the_expected_value_type_can_fix_the_element_parameter() {
    let source = "\
        struct Grid { value: i32 } \
        impl<I, T> Index<I, T> for Grid { \
            func index(self: *Grid, index: I) -> *T { self.index(index) } \
        } \
        func main() -> i32 { let grid = Grid { value: 1 }; grid[0] }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let index = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, index)),
        "i32"
    );
    let Some(IndexProtocol::Custom {
        element_ty, call, ..
    }) = results.index_protocol(index)
    else {
        panic!("the read uses the custom protocol");
    };
    assert_eq!(ty_text_with(&fixture, &hir, *element_ty), "i32");
    assert_eq!(ty_text_with(&fixture, &hir, call.method.arg_tys[0]), "i32");
}

#[test]
fn an_unconstrained_element_type_is_e3055() {
    let source = "\
        struct Grid { value: i32 } \
        impl<I, T> Index<I, T> for Grid { \
            func index(self: *Grid, index: I) -> *T { self.index(index) } \
        } \
        func main() { let grid = Grid { value: 1 }; let _ = grid[0]; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3055", "cannot infer");
}

#[test]
fn an_outer_reference_impl_wins_over_the_value_impl() {
    let source = format!(
        "{GRID} \
         impl Index<usize, i32> for *Grid {{ \
             func index(self: **Grid, index: usize) -> *i32 {{ &(**self).value }} \
         }} \
         func main() -> i32 {{ \
             let grid = Grid {{ value: 7 }}; \
             let pointer = &grid; \
             pointer[0] \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = custom(&results, tail(&hir, body));
    assert_eq!(
        ty_text_with(&fixture, &hir, call.method.receiver_ty),
        "*Grid"
    );
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::SharedBorrow);
}

#[test]
fn a_dyn_index_receiver_uses_vtable_dispatch() {
    let source = format!("{GRID} func read(grid: *dyn Index<usize, i32>) -> i32 {{ grid[0] }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "read");
    let index = tail(&hir, body);
    let call = custom(&results, index);
    assert!(matches!(call.method.kind, MethodKind::Dynamic { .. }));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::None);
    assert!(
        call.instance.is_none(),
        "dynamic dispatch has no static instance"
    );
    assert_eq!(ty_text_with(&fixture, &hir, call.method.ret_ty), "*i32");
}

#[test]
fn reading_a_non_copy_element_through_the_protocol_is_rejected() {
    let source = "\
        struct Item { value: i32 } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        func main() { let grid = Grid { item: Item { value: 1 } }; grid[0]; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn borrowing_a_non_copy_element_through_the_protocol_is_allowed() {
    let source = "\
        struct Item { value: i32 } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        func main() { let grid = Grid { item: Item { value: 1 } }; &grid[0]; }";
    check_clean(source);
}

#[test]
fn a_custom_index_base_must_be_a_place() {
    let source = "struct Grid { value: i32 } \
                  impl Index<usize, i32> for Grid { \
                      func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
                  } \
                  func main() -> i32 { Grid { value: 1 }[0] }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2013", "indexing requires a place");
    assert!(
        !fixture.codes().iter().any(|code| code == "E3021"),
        "a structural place error must not cascade into a protocol error"
    );
}

#[test]
fn built_in_array_indexing_keeps_its_resolved_place() {
    let (fixture, hir, results) = check_clean("func main() -> i32 { let a = [1, 2]; a[0] }");
    let (_, body) = function(&fixture, &hir, "main");
    let index = tail(&hir, body);
    assert!(results.index_protocol(index).is_none());
    assert!(results.place(index).is_some());
}

#[test]
fn a_mut_exclusive_receiver_is_reborrowed_for_a_read() {
    let source = format!("{GRID} func read(grid: *mut Grid) -> i32 {{ grid[0] }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "read");
    let call = custom(&results, tail(&hir, body));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::SharedReborrow);
    assert_eq!(
        ty_text_with(&fixture, &hir, call.method.receiver_ty),
        "Grid"
    );
}
