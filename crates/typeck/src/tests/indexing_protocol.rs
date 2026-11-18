//! Custom indexing through the `Index` protocol.

use crate::{IndexProtocol, MethodKind, PlaceProjection, ReceiverAdjust};
use seal_ty::RefKind;

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

fn custom_mut(results: &crate::TypeckResults, expr: seal_hir::HirExprId) -> &crate::ProtocolCall {
    match results.index_protocol(expr) {
        Some(IndexProtocol::Custom {
            mutable: true,
            call,
            ..
        }) => call,
        other => panic!("expected custom writable indexing, got {other:?}"),
    }
}

const GRID_MUT: &str = "\
struct Grid { value: i32 } \
impl Index<usize, i32> for Grid { \
    func index(self: *Grid, index: usize) -> *i32 { &(*self).value } \
} \
impl IndexMut<usize, i32> for Grid { \
    func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
}";

#[test]
fn a_write_selects_index_mut_and_records_an_exclusive_place() {
    let source =
        format!("{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; grid[0] = 2; }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let index = body_statement_target(&hir, body);
    let call = custom_mut(&results, index);
    assert!(matches!(call.method.kind, MethodKind::Trait { .. }));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::ExclusiveBorrow);
    assert!(call.instance.is_some());

    let place = results.place(index).expect("the write resolves a place");
    assert!(matches!(
        place.projections.last(),
        Some(PlaceProjection::CustomIndex { mutable: true, .. })
    ));
    assert_eq!(place.deref, Some(RefKind::Exclusive));
}

#[test]
fn compound_assignment_and_update_use_index_mut() {
    let source = format!(
        "{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; grid[0] += 2; grid[0]++; }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let mut writes = 0;
    for expr in body_exprs(&hir, body) {
        if let Some(IndexProtocol::Custom { mutable: true, .. }) = results.index_protocol(expr) {
            writes += 1;
        }
    }
    assert_eq!(writes, 2, "both operations write through the protocol");
}

#[test]
fn an_exclusive_borrow_of_an_indexed_element_uses_index_mut() {
    let source = format!(
        "{GRID_MUT} func main() {{ let mut grid = Grid {{ value: 1 }}; let value = &mut grid[0]; *value = 2; }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let index = index_exprs(&hir, body)[0];
    let call = custom_mut(&results, index);
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::ExclusiveBorrow);
}

#[test]
fn index_only_supports_reads_and_writes_are_e3054() {
    let source = format!("{GRID} func main() {{ let grid = Grid {{ value: 1 }}; grid[0] = 2; }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3054", "cannot write through an index");
}

#[test]
fn index_mut_does_not_imply_index_for_reads() {
    let source = "\
        struct Grid { value: i32 } \
        impl IndexMut<usize, i32> for Grid { \
            func index_mut(self: *mut Grid, index: usize) -> *mut i32 { &mut (*self).value } \
        } \
        func main() -> i32 { let mut grid = Grid { value: 1 }; grid[0] }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3021", "cannot index a value");
}

#[test]
fn writing_an_indexed_element_requires_a_mutable_base() {
    let source =
        format!("{GRID_MUT} func main() {{ let grid = Grid {{ value: 1 }}; grid[0] = 2; }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3017", "cannot borrow");
}

#[test]
fn nested_custom_reads_use_index_at_every_level() {
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
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let reads = index_exprs(&hir, body);
    assert_eq!(reads.len(), 2);
    for expr in reads {
        assert!(matches!(
            results.index_protocol(expr),
            Some(IndexProtocol::Custom { mutable: false, .. })
        ));
    }
}

#[test]
fn a_field_after_a_custom_index_keeps_the_custom_projection() {
    let source = "\
        struct Item { value: i32 } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        func main() -> i32 { let grid = Grid { item: Item { value: 7 } }; grid[0].value }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let field = tail(&hir, body);
    let place = results.place(field).expect("the field resolves a place");
    assert!(
        place.contains_custom_index(),
        "the field applies on top of the custom index"
    );
    assert!(!place.contains_deref() || place.deref == Some(RefKind::Shared));
}

#[test]
fn an_exclusive_method_after_a_custom_index_selects_index_mut() {
    let source = "\
        struct Item { value: i32 } \
        impl Item { func bump(self: *mut Item) { (*self).value += 1; } } \
        struct Grid { item: Item } \
        impl Index<usize, Item> for Grid { \
            func index(self: *Grid, index: usize) -> *Item { &(*self).item } \
        } \
        impl IndexMut<usize, Item> for Grid { \
            func index_mut(self: *mut Grid, index: usize) -> *mut Item { &mut (*self).item } \
        } \
        func main() { let mut grid = Grid { item: Item { value: 1 } }; grid[0].bump(); }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let method = body_exprs(&hir, body)
        .into_iter()
        .find(|expr| matches!(hir.expr(*expr).kind, seal_hir::ExprKind::MethodCall { .. }))
        .expect("the body calls a method");
    assert!(results.method(method).is_some());
    let receiver = receiver_of(&hir, method);
    assert!(matches!(
        results.index_protocol(receiver),
        Some(IndexProtocol::Custom { mutable: true, .. })
    ));
}

#[test]
fn an_outer_pointer_index_mut_is_selected_over_the_value_impl() {
    let source = "\
        struct Grid { value: i32 } \
        impl IndexMut<usize, i32> for *mut Grid { \
            func index_mut(self: *mut *mut Grid, index: usize) -> *mut i32 { &mut (**self).value } \
        } \
        func main() { let mut grid = Grid { value: 1 }; let mut pointer: *mut Grid = &mut grid; pointer[0] = 2; }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let index = index_exprs(&hir, body)[0];
    let call = custom_mut(&results, index);
    assert_eq!(
        ty_text_with(&fixture, &hir, call.method.receiver_ty),
        "*mut Grid"
    );
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::ExclusiveBorrow);
}

#[test]
fn a_dyn_index_mut_receiver_uses_vtable_dispatch() {
    let source = "\
        func write(grid: *mut dyn IndexMut<usize, i32>) { grid[0] = 1; }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "write");
    let index = index_exprs(&hir, body)[0];
    let call = custom_mut(&results, index);
    assert!(matches!(call.method.kind, MethodKind::Dynamic { .. }));
    assert_eq!(call.method.receiver_adjust, ReceiverAdjust::None);
    assert!(call.instance.is_none());
}

fn body_exprs(hir: &seal_hir::Hir, body: &seal_hir::Body) -> Vec<seal_hir::HirExprId> {
    let mut exprs = Vec::new();
    for statement in &hir.block(body.root).statements {
        collect_statement(hir, hir.stmt(*statement), &mut exprs);
    }
    if let Some(tail) = hir.block(body.root).tail {
        collect_expr(hir, tail, &mut exprs);
    }
    exprs
}

fn collect_statement(
    hir: &seal_hir::Hir,
    statement: &seal_hir::Stmt,
    exprs: &mut Vec<seal_hir::HirExprId>,
) {
    use seal_hir::StmtKind;
    match &statement.kind {
        StmtKind::Expr(expr)
        | StmtKind::Let {
            init: Some(expr), ..
        } => {
            collect_expr(hir, *expr, exprs);
        }
        StmtKind::Let { init: None, .. }
        | StmtKind::Return(None)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Error => {}
        _ => {}
    }
}

fn collect_expr(
    hir: &seal_hir::Hir,
    expr: seal_hir::HirExprId,
    exprs: &mut Vec<seal_hir::HirExprId>,
) {
    exprs.push(expr);
    use seal_hir::ExprKind;
    match &hir.expr(expr).kind {
        ExprKind::Index { base, index, .. } => {
            collect_expr(hir, *base, exprs);
            collect_expr(hir, *index, exprs);
        }
        ExprKind::Field { base, .. } | ExprKind::TupleField { base, .. } => {
            collect_expr(hir, *base, exprs)
        }
        ExprKind::Borrow { operand, .. } | ExprKind::Deref { operand } => {
            collect_expr(hir, *operand, exprs)
        }
        ExprKind::Assign { target, value, .. } => {
            collect_expr(hir, *target, exprs);
            collect_expr(hir, *value, exprs);
        }
        ExprKind::Update { target, .. } => collect_expr(hir, *target, exprs),
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_expr(hir, *receiver, exprs);
            for arg in args {
                collect_expr(hir, *arg, exprs);
            }
        }
        _ => {}
    }
}

fn index_exprs(hir: &seal_hir::Hir, body: &seal_hir::Body) -> Vec<seal_hir::HirExprId> {
    body_exprs(hir, body)
        .into_iter()
        .filter(|expr| matches!(hir.expr(*expr).kind, seal_hir::ExprKind::Index { .. }))
        .collect()
}

fn body_statement_target(hir: &seal_hir::Hir, body: &seal_hir::Body) -> seal_hir::HirExprId {
    use seal_hir::StmtKind;
    hir.block(body.root)
        .statements
        .iter()
        .find_map(|statement| match &hir.stmt(*statement).kind {
            StmtKind::Expr(expr) => match &hir.expr(*expr).kind {
                seal_hir::ExprKind::Assign { target, .. } => Some(*target),
                _ => None,
            },
            _ => None,
        })
        .expect("the body has an assignment")
}

fn receiver_of(hir: &seal_hir::Hir, expr: seal_hir::HirExprId) -> seal_hir::HirExprId {
    match &hir.expr(expr).kind {
        seal_hir::ExprKind::MethodCall { receiver, .. } => *receiver,
        other => panic!("expected a method call, got {other:?}"),
    }
}
