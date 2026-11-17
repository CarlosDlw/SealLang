//! Match expressions and pattern checking.

use seal_ast::{Literal, PatternLiteral};
use seal_def::DefId;
use seal_hir::{HirExprId, HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;
use seal_hash::FxHashSet;

impl Checker<'_> {
    /// Checks a `match` expression.
    ///
    /// The scrutinee is evaluated and typed once. Every arm pattern is checked
    /// against that type, bindings receive their types and are marked
    /// initialized, guards must be `bool`, and arm bodies share one result type.
    /// Coverage is checked on the patterns alone: a guard can fail, so it never
    /// makes an arm exhaustive.
    pub(super) fn check_match(
        &mut self,
        scrutinee: HirExprId,
        arms: &[MatchArm],
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let scrutinee_ty = self.infer_expr(scrutinee, None, flow);
        let start = flow.clone();

        let mut result_expected = expected;
        let mut outcomes: Vec<(TyId, Flow)> = Vec::with_capacity(arms.len());
        for arm in arms {
            let mut arm_flow = start.clone();
            self.check_pattern(arm.pattern, scrutinee_ty, &mut arm_flow);

            if let Some(guard) = arm.guard {
                let mut moves = FxHashSet::default();
                self.collect_noncopy_bindings(arm.pattern, &mut moves);
                let previous = self.guard_moves.replace(moves);
                self.check_condition(guard, &mut arm_flow);
                self.guard_moves = previous;
            }

            let body_ty = match result_expected {
                Some(expected) => {
                    self.check_expr(arm.body, expected, &mut arm_flow);
                    expected
                }
                None => {
                    let ty = self.infer_expr(arm.body, None, &mut arm_flow);
                    if !self.is_never(ty) {
                        result_expected = Some(ty);
                    }
                    ty
                }
            };
            outcomes.push((body_ty, arm_flow));
        }

        let result_ty = result_expected.unwrap_or_else(|| {
            outcomes
                .iter()
                .map(|(ty, _)| *ty)
                .find(|ty| !self.is_never(*ty))
                .unwrap_or_else(|| self.never())
        });

        let exhaustive =
            !self.is_error(scrutinee_ty) && self.check_match_coverage(scrutinee_ty, arms, span);

        let mut continuing: Vec<Flow> = outcomes
            .into_iter()
            .filter_map(|(_, flow)| (!flow.diverges).then_some(flow))
            .collect();
        if !exhaustive {
            continuing.push(start);
        }

        if continuing.is_empty() {
            flow.diverges = true;
        } else {
            let mut merged = continuing.remove(0);
            for other in continuing {
                let previous = merged.clone();
                merged.merge(previous, other);
            }
            *flow = merged;
        }

        result_ty
    }

    /// Checks a pattern against the scrutinee type and records its bindings.
    fn check_pattern(&mut self, pattern: HirPatternId, expected: TyId, flow: &mut Flow) {
        let (span, kind) = {
            let pattern = self.hir.pattern(pattern);
            (pattern.span, pattern.kind.clone())
        };

        match kind {
            PatternKind::Wildcard | PatternKind::Error => {}
            PatternKind::Binding { def, .. } => {
                self.results.record_local(def, expected);
                flow.assign(def, &[]);
            }
            PatternKind::Literal(literal) => self.check_pattern_literal(literal, expected, span),
            PatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let Some(adt_id) = self.hir.adt(adt) else {
                    return;
                };
                let pattern_ty = self.hir.ty_ctxt().adt_ty(adt_id);
                if !self.is_compatible(expected, pattern_ty) {
                    self.mismatch(span, expected, pattern_ty);
                }
                let Some(definition) = self.hir.ty_ctxt().variant(adt_id, variant) else {
                    return;
                };
                for (field, field_pattern) in fields {
                    let Some(field) = definition.field(field) else {
                        continue;
                    };
                    let field_ty = field.ty;
                    self.check_pattern(field_pattern, field_ty, flow);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.check_pattern(alternative, expected, flow);
                }
            }
        }
    }

    fn check_pattern_literal(&mut self, literal: PatternLiteral, expected: TyId, span: Span) {
        let (literal, allow_signed_min) = match literal {
            PatternLiteral::Int {
                value,
                negative,
                suffix,
            } => (
                Literal::Int {
                    value,
                    base: 10,
                    suffix,
                },
                negative,
            ),
            PatternLiteral::Float { value, suffix } => (Literal::Float { value, suffix }, false),
            PatternLiteral::Char(value) => (Literal::Char(value), false),
            PatternLiteral::Bool(value) => (Literal::Bool(value), false),
        };
        let ty = self.literal_ty(literal, Some(expected), span, allow_signed_min);
        self.mismatch(span, expected, ty);
    }

    /// Collects the non-`Copy` bindings of a pattern, for guard checking.
    fn collect_noncopy_bindings(&self, pattern: HirPatternId, out: &mut FxHashSet<DefId>) {
        match &self.hir.pattern(pattern).kind {
            PatternKind::Binding { def, .. } => {
                let ty = self.local_type(*def);
                if !self.is_copy(ty) {
                    out.insert(*def);
                }
            }
            PatternKind::Variant { fields, .. } => {
                for (_, field) in fields {
                    self.collect_noncopy_bindings(*field, out);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.collect_noncopy_bindings(*alternative, out);
                }
            }
            PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Error => {}
        }
    }
}
