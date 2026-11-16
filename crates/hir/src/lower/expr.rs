//! Expression lowering.

use seal_ast::{ExprId as AstExprId, ExprKind as AstExprKind};

use super::Lowering;
use crate::expr::{Expr, ExprKind};
use crate::id::HirExprId;

impl Lowering<'_> {
    /// Lowers one expression. `Paren` is removed: the tree already makes
    /// grouping explicit.
    pub(super) fn lower_expr(&mut self, expr: AstExprId) -> HirExprId {
        let (span, kind) = {
            let expr = self.ast.expr(expr);
            (expr.span, expr.kind.clone())
        };

        let kind = match kind {
            AstExprKind::Lit(literal) => ExprKind::Lit(literal),
            AstExprKind::Path(name) => match self.resolve(name, span) {
                Some(res) => ExprKind::Path(res),
                None => ExprKind::Error,
            },
            AstExprKind::Unary { op, operand } => ExprKind::Unary {
                op,
                operand: self.lower_expr(operand),
            },
            AstExprKind::Borrow { operand } => ExprKind::Borrow {
                operand: self.lower_expr(operand),
            },
            AstExprKind::Deref { operand } => ExprKind::Deref {
                operand: self.lower_expr(operand),
            },
            AstExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
                op,
                lhs: self.lower_expr(lhs),
                rhs: self.lower_expr(rhs),
            },
            AstExprKind::Assign { op, target, value } => ExprKind::Assign {
                op,
                target: self.lower_expr(target),
                value: self.lower_expr(value),
            },
            AstExprKind::Update {
                op,
                operand,
                postfix,
            } => ExprKind::Update {
                op,
                target: self.lower_expr(operand),
                postfix,
            },
            AstExprKind::Call { callee, args } => ExprKind::Call {
                callee: self.lower_expr(callee),
                args: args.iter().map(|arg| self.lower_expr(*arg)).collect(),
            },
            AstExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => ExprKind::If {
                condition: self.lower_expr(condition),
                then_branch: self.lower_block(then_branch),
                else_branch: else_branch.map(|branch| self.lower_block(branch)),
            },
            AstExprKind::Paren(inner) => return self.lower_expr(inner),
            AstExprKind::Error => ExprKind::Error,
        };

        self.hir.push_expr(Expr { kind, span })
    }
}
