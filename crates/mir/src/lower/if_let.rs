//! Lowering `if let` expressions.
//!
//! The scrutinee is evaluated once into a temporary. The pattern tests that
//! temporary and jumps to the else block when it fails. Bindings are assigned
//! only on success, and the then block is the only place they are visible.

use seal_hir::{HirBlockId, HirExprId, HirPatternId};
use seal_span::Span;

use super::BodyLowering;
use crate::id::BasicBlockId;
use crate::operand::Constant;
use crate::place::Place;

impl BodyLowering<'_> {
    /// Lowers one `if let` expression.
    pub(super) fn lower_if_let(
        &mut self,
        pattern: HirPatternId,
        scrutinee: HirExprId,
        then_branch: HirBlockId,
        else_branch: Option<HirBlockId>,
        dest: Option<Place>,
        span: Span,
    ) {
        let ty = self.expr_ty(scrutinee);
        let subject = self.new_temp(ty, span);
        self.lower_expr_into(scrutinee, Some(subject.clone()));

        let join = self.new_block();
        let failure = self.new_block();
        self.declare_pattern_bindings(pattern);

        let mut alternatives = self.expand_pattern(pattern);
        if alternatives.is_empty() {
            self.terminate_goto(failure, span);
        } else {
            let mut entry = self
                .current
                .expect("an if let expression follows a reachable scrutinee");
            let mut failures: Vec<BasicBlockId> =
                Vec::with_capacity(alternatives.len().saturating_sub(1));
            for _ in 0..alternatives.len().saturating_sub(1) {
                failures.push(self.new_block());
            }

            for (position, alternative) in alternatives.drain(..).enumerate() {
                let fail = failures.get(position).copied().unwrap_or(failure);
                let mut bindings = Vec::new();
                self.current = Some(entry);
                self.lower_flat_pattern(&alternative, subject.clone(), ty, fail, &mut bindings);
                self.bind_all(&bindings, span);
                self.lower_block(then_branch, dest.clone());
                if self.current.is_some() {
                    self.terminate_goto(join, span);
                }
                entry = fail;
            }
        }

        self.current = Some(failure);
        if let Some(else_branch) = else_branch {
            self.lower_block(else_branch, dest);
        } else if let Some(dest) = dest {
            self.assign_constant(dest, Constant::Unit, span);
        }
        if self.current.is_some() {
            self.terminate_goto(join, span);
        }

        self.current = Some(join);
    }
}
