//! End-to-end lowering of the example programs.

use std::path::PathBuf;

use crate::{ExprKind, Hir, HirBlockId, HirExprId, HirStmtId, StmtKind};
use seal_def::DefKind;

use super::support::{assert_invariants, lower_clean};

const EXAMPLES: [&str; 35] = [
    "0001_main.sl",
    "0002_variables.sl",
    "0003_operators.sl",
    "004_conditionals.sl",
    "005_while_loop.sl",
    "006_structs.sl",
    "007_runtime_panic.sl",
    "008_exclusive_references.sl",
    "010_slices.sl",
    "011_methods.sl",
    "012_user_methods.sl",
    "013_method_borrows.sl",
    "014_method_namespaces.sl",
    "015_traits.sl",
    "016_trait_receivers.sl",
    "017_enums.sl",
    "018_tuple_variants.sl",
    "019_match.sl",
    "020_match_advanced.sl",
    "021_match_slice.sl",
    "022_match_reference.sl",
    "023_for.sl",
    "024_if_let.sl",
    "025_while_let.sl",
    "026_tuples.sl",
    "027_generic_functions.sl",
    "028_generic_structs.sl",
    "029_generic_enums.sl",
    "030_generic_traits.sl",
    "031_generic_recursion.sl",
    "035_numeric_casts.sl",
    "036_default_methods.sl",
    "039_const_static.sl",
    "040_global_data.sl",
    "041_global_mutation.sl",
];

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn inspect(hir: &Hir) -> (Vec<StmtKind>, Vec<ExprKind>) {
    let mut stmts = Vec::new();
    let mut exprs = Vec::new();
    for body in hir.bodies().iter() {
        collect_block(hir, body.root, &mut stmts, &mut exprs);
    }
    (stmts, exprs)
}

fn collect_block(
    hir: &Hir,
    block: HirBlockId,
    stmts: &mut Vec<StmtKind>,
    exprs: &mut Vec<ExprKind>,
) {
    for statement in &hir.block(block).statements {
        collect_stmt(hir, *statement, stmts, exprs);
    }
    if let Some(tail) = hir.block(block).tail {
        collect_expr(hir, tail, stmts, exprs);
    }
}

fn collect_stmt(
    hir: &Hir,
    statement: HirStmtId,
    stmts: &mut Vec<StmtKind>,
    exprs: &mut Vec<ExprKind>,
) {
    let kind = hir.stmt(statement).kind.clone();
    match &kind {
        StmtKind::Let { init, .. } => {
            if let Some(init) = init {
                collect_expr(hir, *init, stmts, exprs);
            }
        }
        StmtKind::Return(Some(value)) => collect_expr(hir, *value, stmts, exprs),
        StmtKind::While { condition, body } => {
            collect_expr(hir, *condition, stmts, exprs);
            collect_block(hir, *body, stmts, exprs);
        }
        StmtKind::WhileLet {
            scrutinee, body, ..
        } => {
            collect_expr(hir, *scrutinee, stmts, exprs);
            collect_block(hir, *body, stmts, exprs);
        }
        StmtKind::For { source, body, .. } => {
            match source {
                crate::ForSource::Range { start, end, .. } => {
                    collect_expr(hir, *start, stmts, exprs);
                    collect_expr(hir, *end, stmts, exprs);
                }
                crate::ForSource::Expr(expr) => collect_expr(hir, *expr, stmts, exprs),
            }
            collect_block(hir, *body, stmts, exprs);
        }
        StmtKind::Expr(expr) => collect_expr(hir, *expr, stmts, exprs),
        StmtKind::Const { .. } | StmtKind::Static { .. } => {}
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
    }
    stmts.push(kind);
}

fn collect_expr(hir: &Hir, expr: HirExprId, stmts: &mut Vec<StmtKind>, exprs: &mut Vec<ExprKind>) {
    let kind = hir.expr(expr).kind.clone();
    match &kind {
        ExprKind::Unary { operand, .. }
        | ExprKind::Borrow { operand, .. }
        | ExprKind::Deref { operand } => collect_expr(hir, *operand, stmts, exprs),
        ExprKind::Field { base, .. } | ExprKind::TupleField { base, .. } => {
            collect_expr(hir, *base, stmts, exprs)
        }
        ExprKind::Tuple { elements } => {
            for element in elements {
                collect_expr(hir, *element, stmts, exprs);
            }
        }
        ExprKind::Array { elements } => {
            for element in elements {
                collect_expr(hir, *element, stmts, exprs);
            }
        }
        ExprKind::ArrayRepeat { value, .. } => collect_expr(hir, *value, stmts, exprs),
        ExprKind::Index { base, index } => {
            collect_expr(hir, *base, stmts, exprs);
            collect_expr(hir, *index, stmts, exprs);
        }
        ExprKind::Slice { base, start, end } => {
            collect_expr(hir, *base, stmts, exprs);
            for bound in start.iter().chain(end.iter()) {
                collect_expr(hir, *bound, stmts, exprs);
            }
        }
        ExprKind::SliceRef {
            base, start, end, ..
        } => {
            collect_expr(hir, *base, stmts, exprs);
            for bound in start.iter().chain(end.iter()) {
                collect_expr(hir, *bound, stmts, exprs);
            }
        }
        ExprKind::Len { operand } => collect_expr(hir, *operand, stmts, exprs),
        ExprKind::Struct { fields, .. } | ExprKind::Variant { fields, .. } => {
            for (_, value) in fields {
                collect_expr(hir, *value, stmts, exprs);
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            collect_expr(hir, *lhs, stmts, exprs);
            collect_expr(hir, *rhs, stmts, exprs);
        }
        ExprKind::Cast { operand, .. } => collect_expr(hir, *operand, stmts, exprs),
        ExprKind::Assign { target, value, .. } => {
            collect_expr(hir, *target, stmts, exprs);
            collect_expr(hir, *value, stmts, exprs);
        }
        ExprKind::Update { target, .. } => collect_expr(hir, *target, stmts, exprs),
        ExprKind::Call { callee, args } => {
            collect_expr(hir, *callee, stmts, exprs);
            for arg in args {
                collect_expr(hir, *arg, stmts, exprs);
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            collect_expr(hir, *receiver, stmts, exprs);
            for arg in args {
                collect_expr(hir, *arg, stmts, exprs);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            collect_expr(hir, *scrutinee, stmts, exprs);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    collect_expr(hir, guard, stmts, exprs);
                }
                collect_expr(hir, arm.body, stmts, exprs);
            }
        }
        ExprKind::Block(block) | ExprKind::Unsafe { block } => {
            collect_block(hir, *block, stmts, exprs)
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_expr(hir, *condition, stmts, exprs);
            collect_block(hir, *then_branch, stmts, exprs);
            if let Some(else_branch) = else_branch {
                collect_block(hir, *else_branch, stmts, exprs);
            }
        }
        ExprKind::IfLet {
            scrutinee,
            then_branch,
            else_branch,
            ..
        } => {
            collect_expr(hir, *scrutinee, stmts, exprs);
            collect_block(hir, *then_branch, stmts, exprs);
            if let Some(else_branch) = else_branch {
                collect_block(hir, *else_branch, stmts, exprs);
            }
        }
        ExprKind::Lit(_) | ExprKind::Path { .. } | ExprKind::Error => {}
    }
    exprs.push(kind);
}

#[test]
fn all_examples_lower_without_diagnostics() {
    for name in EXAMPLES {
        let (fixture, hir) = lower_clean(&source(name));
        assert_invariants(&fixture, &hir);
        for (id, data) in hir.definitions().iter_enumerated() {
            if data.kind == DefKind::Function {
                assert!(
                    hir.body_of(id).is_some(),
                    "function `{}` in {name} has no body",
                    fixture.symbol(data.name)
                );
            }
        }
    }
}

#[test]
fn conditionals_example_lowers_if_expressions() {
    let (fixture, hir) = lower_clean(&source("004_conditionals.sl"));
    assert_invariants(&fixture, &hir);

    let (_, exprs) = inspect(&hir);
    assert!(
        exprs.iter().any(|kind| matches!(kind, ExprKind::If { .. })),
        "expected at least one lowered if"
    );
    assert!(
        exprs.iter().any(|kind| {
            matches!(
                kind,
                ExprKind::Call { callee, .. }
                    if matches!(&hir.expr(*callee).kind, ExprKind::Path { .. })
            )
        }),
        "expected a call with a resolved callee"
    );
}

#[test]
fn while_loop_example_lowers_loop_control() {
    let (fixture, hir) = lower_clean(&source("005_while_loop.sl"));
    assert_invariants(&fixture, &hir);

    let (stmts, _) = inspect(&hir);
    assert!(
        stmts
            .iter()
            .any(|kind| matches!(kind, StmtKind::While { .. })),
        "expected a lowered while"
    );
    assert!(
        stmts.iter().any(|kind| matches!(kind, StmtKind::Break)),
        "expected a lowered break"
    );
    assert!(
        stmts.iter().any(|kind| matches!(kind, StmtKind::Continue)),
        "expected a lowered continue"
    );
}
