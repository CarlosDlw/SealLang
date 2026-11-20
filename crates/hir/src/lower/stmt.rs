//! Statement lowering.

use seal_ast::{ForSource as AstForSource, StmtId as AstStmtId, StmtKind as AstStmtKind};
use seal_def::{DefData, DefKind};
use seal_error_codes as codes;

use super::Lowering;
use crate::body::{BodyKind, ForSource, Stmt, StmtKind};
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
                let local = self.define(DefData::new(
                    DefKind::Binding,
                    binding.name,
                    binding.name_span,
                    span,
                    self.function,
                    binding.mutable,
                ));
                self.scopes.insert(binding.name, local);
                StmtKind::Let { local, ty, init }
            }
            AstStmtKind::Const(declaration) => {
                let barrier = self.capture_context();
                let definition = self.define_local_value(
                    DefKind::Const,
                    declaration.name,
                    declaration.name_span,
                    span,
                    false,
                );
                self.lower_value_body(
                    definition,
                    declaration.ty,
                    declaration.init,
                    BodyKind::Const,
                    Some(barrier),
                );
                self.declare_local_value(definition, declaration.name, declaration.name_span);
                StmtKind::Const { def: definition }
            }
            AstStmtKind::Static(declaration) => {
                let barrier = self.capture_context();
                let definition = self.define_local_value(
                    DefKind::Static,
                    declaration.name,
                    declaration.name_span,
                    span,
                    declaration.mutable,
                );
                self.lower_value_body(
                    definition,
                    declaration.ty,
                    declaration.init,
                    BodyKind::Static {
                        mutable: declaration.mutable,
                    },
                    Some(barrier),
                );
                self.declare_local_value(definition, declaration.name, declaration.name_span);
                StmtKind::Static { def: definition }
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
            AstStmtKind::WhileLet {
                pattern,
                scrutinee,
                body,
            } => {
                // The scrutinee is evaluated before the pattern is in scope, so
                // it cannot read this iteration's bindings.
                let scrutinee = self.lower_expr(scrutinee);
                self.scopes.push();
                let pattern = self.lower_arm_pattern(pattern);
                self.loop_depth += 1;
                let body = self.lower_block(body);
                self.loop_depth -= 1;
                self.scopes.pop();
                StmtKind::WhileLet {
                    pattern,
                    scrutinee,
                    body,
                }
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
