//! Statement lowering.

use seal_ast::{ForSource as AstForSource, StmtId as AstStmtId, StmtKind as AstStmtKind};
use seal_def::{DefData, DefKind};
use seal_error_codes as codes;

use super::Lowering;
use crate::body::{ForSource, Stmt, StmtKind};
use crate::id::HirStmtId;

impl Lowering<'_> {
    /// Lowers one statement.
    pub(super) fn lower_stmt(&mut self, statement: AstStmtId) -> HirStmtId {
        let (span, kind) = {
            let statement = self.ast.stmt(statement);
            (statement.span, statement.kind.clone())
        };

        let kind = match kind {
            AstStmtKind::Let(binding) => {
                let ty = binding.ty.map(|ty| self.lower_type(ty));
                // The binding is declared after its initializer is lowered, so
                // `let x = x;` reads the outer `x`.
                let init = binding.init.map(|init| self.lower_expr(init));
                let local = self.hir.define(DefData {
                    kind: DefKind::Binding,
                    name: binding.name,
                    name_span: binding.name_span,
                    span,
                    parent: self.function,
                    mutable: binding.mutable,
                });
                self.scopes.insert(binding.name, local);
                StmtKind::Let { local, ty, init }
            }
            AstStmtKind::Return(value) => {
                StmtKind::Return(value.map(|value| self.lower_expr(value)))
            }
            AstStmtKind::While { condition, body } => {
                let condition = self.lower_expr(condition);
                self.loop_depth += 1;
                let body = self.lower_block(body);
                self.loop_depth -= 1;
                StmtKind::While { condition, body }
            }
            AstStmtKind::For {
                pattern,
                source,
                body,
            } => {
                // The iterable is evaluated before the pattern is in scope, so
                // a bound such as `limit` does not see the loop binding.
                let source = match source {
                    AstForSource::Range {
                        start,
                        end,
                        inclusive,
                    } => ForSource::Range {
                        start: self.lower_expr(start),
                        end: self.lower_expr(end),
                        inclusive,
                    },
                    AstForSource::Expr(expr) => ForSource::Expr(self.lower_expr(expr)),
                };
                self.scopes.push();
                let pattern = self.lower_arm_pattern(pattern);
                self.loop_depth += 1;
                let body = self.lower_block(body);
                self.loop_depth -= 1;
                self.scopes.pop();
                StmtKind::For {
                    pattern,
                    source,
                    body,
                }
            }
            AstStmtKind::Break => {
                if self.loop_depth == 0 {
                    self.error(codes::E2003, span, "`break` outside of a loop")
                        .help("`break` can only be used inside a loop")
                        .emit();
                }
                StmtKind::Break
            }
            AstStmtKind::Continue => {
                if self.loop_depth == 0 {
                    self.error(codes::E2004, span, "`continue` outside of a loop")
                        .help("`continue` can only be used inside a loop")
                        .emit();
                }
                StmtKind::Continue
            }
            AstStmtKind::Expr(expr) => StmtKind::Expr(self.lower_expr(expr)),
            AstStmtKind::Error => StmtKind::Error,
        };

        self.hir.push_stmt(Stmt { kind, span })
    }
}
