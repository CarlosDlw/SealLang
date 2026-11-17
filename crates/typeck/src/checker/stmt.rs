//! Block and statement checking.

use seal_hir::{HirBlockId, HirStmtId, StmtKind};
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

pub(crate) struct BlockOutcome {
    pub(crate) ty: TyId,
    pub(crate) flow: Flow,
}

pub(super) fn check_block(
    checker: &mut Checker<'_>,
    block: HirBlockId,
    expected: Option<TyId>,
    mut flow: Flow,
) -> BlockOutcome {
    let statement_count = checker.hir.block(block).statements.len();
    for index in 0..statement_count {
        let statement = checker.hir.block(block).statements[index];
        flow = check_stmt(checker, statement, flow);
    }

    let tail = checker.hir.block(block).tail;
    let ty = match tail {
        Some(tail) => {
            let found = checker.infer_expr(tail, expected, &mut flow);
            if let Some(expected) = expected {
                checker.mismatch(checker.hir.expr(tail).span, expected, found);
                if !checker.is_compatible(expected, found) {
                    checker.error_ty()
                } else {
                    found
                }
            } else {
                found
            }
        }
        None if flow.diverges => checker.never(),
        None => checker.unit(),
    };

    if tail.is_none()
        && let Some(expected) = expected
        && ty != expected
    {
        checker.mismatch(checker.hir.block(block).span, expected, ty);
        let ty = if checker.is_compatible(expected, ty) {
            ty
        } else {
            checker.error_ty()
        };
        return BlockOutcome { ty, flow };
    }

    BlockOutcome { ty, flow }
}

fn check_stmt(checker: &mut Checker<'_>, statement: HirStmtId, mut flow: Flow) -> Flow {
    let (span, kind) = {
        let statement = checker.hir.stmt(statement);
        (statement.span, statement.kind.clone())
    };

    match kind {
        StmtKind::Let { local, ty, init } => {
            let declared = ty.map(|ty| checker.storage_ty(ty, span, "binding"));
            let local_ty = match (declared, init) {
                (Some(declared), Some(init)) => {
                    checker.check_expr(init, declared, &mut flow);
                    declared
                }
                (Some(declared), None) => declared,
                (None, Some(init)) => {
                    let inferred = checker.infer_expr(init, None, &mut flow);
                    checker.storage_ty(inferred, span, "binding")
                }
                (None, None) => checker.error_ty(),
            };
            checker.results.record_local(local, local_ty);
            if init.is_some() || declared.is_none() {
                flow.assign(local, &[]);
            }
        }
        StmtKind::Return(value) => {
            match value {
                Some(value) => checker.check_expr(value, checker.ret, &mut flow),
                None if checker.ret != checker.unit() && !checker.is_error(checker.ret) => {
                    checker
                        .dcx
                        .error("mismatched types")
                        .code(seal_error_codes::E3001.diag_code())
                        .primary(
                            span,
                            format!("expected `{}`, found `()`", checker.ty_text(checker.ret)),
                        )
                        .help("return a value of the declared type")
                        .emit();
                }
                None => {}
            }
            flow.diverges = true;
        }
        StmtKind::While { condition, body } => {
            checker.check_condition(condition, &mut flow);
            let _body = checker.check_block(body, None, flow.clone());
        }
        StmtKind::WhileLet {
            pattern,
            scrutinee,
            body,
        } => {
            let scrutinee_ty = checker.infer_expr(scrutinee, None, &mut flow);
            let mut body_flow = flow.clone();
            checker.check_pattern(pattern, scrutinee_ty, &mut body_flow);
            let _body = checker.check_block(body, None, body_flow);
        }
        StmtKind::For {
            pattern,
            source,
            body,
        } => {
            flow = checker.check_for(pattern, source, body, span, flow);
        }
        StmtKind::Break | StmtKind::Continue => {
            flow.diverges = true;
        }
        StmtKind::Expr(expr) => {
            checker.infer_expr(expr, None, &mut flow);
        }
        StmtKind::Error => {}
    }

    flow
}
