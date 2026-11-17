//! Lowering `while let` loops.
//!
//! The scrutinee is evaluated at the loop head on every iteration. The pattern
//! tests that value and jumps to the exit when it fails; a successful match
//! binds its names and runs the body, whose backedge returns to the head.
//! `continue` therefore reevaluates the scrutinee, and `break` leaves the loop.

use seal_hir::{HirBlockId, HirExprId, HirPatternId};
use seal_span::Span;

use super::BodyLowering;
use crate::id::BasicBlockId;

impl BodyLowering<'_> {
    /// Lowers one `while let` statement.
    pub(super) fn lower_while_let(
        &mut self,
        pattern: HirPatternId,
        scrutinee: HirExprId,
        body: HirBlockId,
        span: Span,
    ) {
        let condition = self.new_block();
        let exit = self.new_block();

        self.terminate_goto(condition, span);
        self.current = Some(condition);

        let ty = self.expr_ty(scrutinee);
        let subject = self.new_temp(ty, span);
        self.lower_expr_into(scrutinee, Some(subject.clone()));
        self.declare_pattern_bindings(pattern);

        let mut alternatives = self.expand_pattern(pattern);
        if alternatives.is_empty() {
            self.terminate_goto(exit, span);
        } else {
            let mut entry = self
                .current
                .expect("a while let loop follows a reachable scrutinee");
            let mut failures: Vec<BasicBlockId> =
                Vec::with_capacity(alternatives.len().saturating_sub(1));
            for _ in 0..alternatives.len().saturating_sub(1) {
                failures.push(self.new_block());
            }

            for (position, alternative) in alternatives.drain(..).enumerate() {
                let fail = failures.get(position).copied().unwrap_or(exit);
                let mut bindings = Vec::new();
                self.current = Some(entry);
                self.lower_flat_pattern(&alternative, subject.clone(), ty, fail, &mut bindings);
                self.bind_all(&bindings, span);

                self.loops.push(super::LoopTargets {
                    break_to: exit,
                    continue_to: condition,
                });
                self.lower_block(body, None);
                self.loops.pop();
                if self.current.is_some() {
                    self.terminate_goto(condition, span);
                }
                entry = fail;
            }
        }

        self.current = Some(exit);
    }
}
