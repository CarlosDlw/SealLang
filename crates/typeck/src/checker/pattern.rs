//! Match expressions and pattern checking.

use seal_ast::{Literal, PatternLiteral};
use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_hir::{HirExprId, HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::Checker;
use super::flow::Flow;

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
    pub(super) fn check_pattern(&mut self, pattern: HirPatternId, expected: TyId, flow: &mut Flow) {
        let (span, kind) = {
            let pattern = self.hir.pattern(pattern);
            (pattern.span, pattern.kind.clone())
        };

        match kind {
            PatternKind::Wildcard | PatternKind::Error => {}
            PatternKind::Binding { def, pattern, .. } => {
                self.results.record_local(def, expected);
                flow.assign(def, &[]);
                if let Some(pattern) = pattern {
                    self.check_pattern(pattern, expected, flow);
                }
            }
            PatternKind::Literal(literal) => self.check_pattern_literal(literal, expected, span),
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => self.check_pattern_range(start, end, inclusive, expected, span),
            PatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let Some(adt_id) = self.hir.adt(adt) else {
                    return;
                };
                let (expected, through_reference) = self.deref_pattern_expected(expected);
                let args = match self.hir.ty_ctxt().kind(expected) {
                    TyKind::Adt {
                        adt: expected_adt,
                        args,
                    } if expected_adt == adt_id => Some(args),
                    _ => None,
                };
                let pattern_ty = match args {
                    Some(args) => self.hir.ty_ctxt().adt_ty_with(adt_id, args),
                    None if self.hir.ty_ctxt().adt_generic_count(adt_id) == 0 => {
                        self.hir.ty_ctxt().adt_ty(adt_id)
                    }
                    None => self.error_ty(),
                };
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
                    let field_ty = match args {
                        Some(args) => self.hir.ty_ctxt().substitute(field.ty, args),
                        None => self.error_ty(),
                    };
                    self.check_pattern(field_pattern, field_ty, flow);
                }
                if through_reference {
                    self.report_reference_pattern_moves(pattern, span);
                }
            }
            PatternKind::Struct { adt, fields } => {
                let Some(adt_id) = self.hir.adt(adt) else {
                    return;
                };
                let (expected, through_reference) = self.deref_pattern_expected(expected);
                let args = match self.hir.ty_ctxt().kind(expected) {
                    TyKind::Adt {
                        adt: expected_adt,
                        args,
                    } if expected_adt == adt_id => Some(args),
                    _ => None,
                };
                let pattern_ty = match args {
                    Some(args) => self.hir.ty_ctxt().adt_ty_with(adt_id, args),
                    None if self.hir.ty_ctxt().adt_generic_count(adt_id) == 0 => {
                        self.hir.ty_ctxt().adt_ty(adt_id)
                    }
                    None => self.error_ty(),
                };
                if !self.is_compatible(expected, pattern_ty) {
                    self.mismatch(span, expected, pattern_ty);
                }
                let definition = self.hir.ty_ctxt().adt(adt_id);
                for (field, field_pattern) in fields {
                    let Some(field) = definition.field(field) else {
                        continue;
                    };
                    let field_ty = match args {
                        Some(args) => self.hir.ty_ctxt().substitute(field.ty, args),
                        None => self.error_ty(),
                    };
                    self.check_pattern(field_pattern, field_ty, flow);
                }
                if through_reference {
                    self.report_reference_pattern_moves(pattern, span);
                }
            }
            PatternKind::Tuple(elements) => {
                self.check_tuple_pattern(&elements, expected, flow, span);
            }
            PatternKind::Array {
                prefix,
                has_rest,
                suffix,
            } => {
                if let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(expected)
                    && let TyKind::Slice { element } = self.hir.ty_ctxt().kind(pointee)
                {
                    let binds = prefix
                        .iter()
                        .chain(suffix.iter())
                        .any(|pattern| self.pattern_binds_anything(*pattern));
                    if binds && !self.is_copy(element) {
                        self.error(
                            codes::E3018,
                            span,
                            format!(
                                "cannot move a `{}` value out of a slice pattern",
                                self.ty_text(element)
                            ),
                        )
                        .help(
                            "the element is not `Copy`; match it with `_` or a literal, or copy it first",
                        )
                        .emit();
                    }
                    for pattern in prefix.iter().chain(suffix.iter()) {
                        self.check_pattern(*pattern, element, flow);
                    }
                    return;
                }
                let (element_ty, length) = match self.hir.ty_ctxt().kind(expected) {
                    TyKind::Array { element, length } => (element, length),
                    TyKind::Error => {
                        for pattern in prefix.iter().chain(suffix.iter()) {
                            self.check_pattern(*pattern, expected, flow);
                        }
                        return;
                    }
                    _ => {
                        if !self.is_error(expected) {
                            self.error(
                                codes::E2049,
                                span,
                                format!(
                                    "an array pattern needs an array, found `{}`",
                                    self.ty_text(expected)
                                ),
                            )
                            .help("match an array value, or use `_`")
                            .emit();
                        }
                        let error = self.error_ty();
                        for pattern in prefix.iter().chain(suffix.iter()) {
                            self.check_pattern(*pattern, error, flow);
                        }
                        return;
                    }
                };
                let fixed = prefix.len() + suffix.len();
                if has_rest {
                    if fixed as u64 > length {
                        self.error(
                            codes::E2049,
                            span,
                            format!(
                                "this pattern needs at least {fixed} element(s), but the array has {length}"
                            ),
                        )
                        .help("remove patterns until they fit, or use `..` for the middle")
                        .emit();
                    }
                } else if fixed as u64 != length {
                    self.error(
                        codes::E2049,
                        span,
                        format!("this pattern has {fixed} element(s), but the array has {length}"),
                    )
                    .help("write one pattern per element, or `..` for a run you ignore")
                    .emit();
                }
                for element in prefix.iter().chain(suffix.iter()) {
                    self.check_pattern(*element, element_ty, flow);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.check_pattern(alternative, expected, flow);
                }
            }
        }
    }

    fn check_tuple_pattern(
        &mut self,
        elements: &[HirPatternId],
        expected: TyId,
        flow: &mut Flow,
        span: Span,
    ) {
        let (expected, through_reference) = self.deref_pattern_expected(expected);
        let declared = match self.hir.ty_ctxt().kind(expected) {
            TyKind::Tuple(tuple) => self.hir.ty_ctxt().tuple_elements(tuple),
            TyKind::Unit => Vec::new(),
            TyKind::Error => {
                let error = self.error_ty();
                for element in elements {
                    self.check_pattern(*element, error, flow);
                }
                return;
            }
            _ => {
                self.error(
                    codes::E3037,
                    span,
                    format!(
                        "a tuple pattern needs a tuple, found `{}`",
                        self.ty_text(expected)
                    ),
                )
                .help("match a tuple value, or use `_`")
                .emit();
                let error = self.error_ty();
                for element in elements {
                    self.check_pattern(*element, error, flow);
                }
                return;
            }
        };

        if declared.len() != elements.len() {
            self.error(
                codes::E3037,
                span,
                format!(
                    "this tuple pattern has {} element(s), but the tuple has {}",
                    elements.len(),
                    declared.len()
                ),
            )
            .help("write one pattern per tuple element, using `_` for ignored values")
            .emit();
        }

        for (position, element) in elements.iter().enumerate() {
            let element_ty = declared
                .get(position)
                .copied()
                .unwrap_or_else(|| self.error_ty());
            self.check_pattern(*element, element_ty, flow);
        }
        if through_reference {
            self.report_reference_moves_in_patterns(elements, span);
        }
    }

    /// Unwraps the references a struct or variant pattern matches through.
    fn deref_pattern_expected(&self, mut ty: TyId) -> (TyId, bool) {
        let mut through_reference = false;
        while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(ty) {
            ty = pointee;
            through_reference = true;
        }
        (ty, through_reference)
    }

    /// Rejects bindings that would move a non-`Copy` value out of a reference
    /// reached by a struct or variant pattern.
    fn report_reference_pattern_moves(&mut self, pattern: HirPatternId, span: Span) {
        self.report_reference_moves_in_patterns(&[pattern], span);
    }

    fn report_reference_moves_in_patterns(&mut self, patterns: &[HirPatternId], span: Span) {
        let mut bindings = FxHashSet::default();
        for pattern in patterns {
            self.collect_noncopy_bindings(*pattern, &mut bindings);
        }
        if let Some(definition) = bindings.into_iter().next() {
            let ty = self.local_type(definition);
            self.error(
                codes::E3018,
                span,
                format!(
                    "cannot move a `{}` value out of a reference in a pattern",
                    self.ty_text(ty)
                ),
            )
            .help("match only `Copy` fields through a reference, or dereference the value first")
            .emit();
        }
    }

    /// Returns `true` when a pattern binds at least one value, which means it
    /// moves the matched value unless its type is `Copy`.
    fn pattern_binds_anything(&self, pattern: HirPatternId) -> bool {
        match &self.hir.pattern(pattern).kind {
            PatternKind::Binding { .. } => true,
            PatternKind::Variant { fields, .. } | PatternKind::Struct { fields, .. } => fields
                .iter()
                .any(|(_, pattern)| self.pattern_binds_anything(*pattern)),
            PatternKind::Tuple(elements) => elements
                .iter()
                .any(|pattern| self.pattern_binds_anything(*pattern)),
            PatternKind::Array { prefix, suffix, .. } => prefix
                .iter()
                .chain(suffix.iter())
                .any(|pattern| self.pattern_binds_anything(*pattern)),
            PatternKind::Or(alternatives) => alternatives
                .iter()
                .any(|alternative| self.pattern_binds_anything(*alternative)),
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Range { .. }
            | PatternKind::Error => false,
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

    /// Checks both endpoints of a range and rejects empty or non-scalar ranges.
    fn check_pattern_range(
        &mut self,
        start: PatternLiteral,
        end: PatternLiteral,
        inclusive: bool,
        expected: TyId,
        span: Span,
    ) {
        let kind = self.hir.ty_ctxt().kind(expected);
        let integer = matches!(kind, TyKind::Int(_) | TyKind::Uint(_));
        let character = matches!(kind, TyKind::Char);
        if !integer && !character {
            if !self.is_error(expected) {
                self.error(
                    codes::E1011,
                    span,
                    format!("a range pattern cannot match `{}`", self.ty_text(expected)),
                )
                .help("range patterns need an integer or `char` value")
                .emit();
            }
            return;
        }

        self.check_range_bound(start, expected, span);
        self.check_range_bound(end, expected, span);

        let empty = match (start, end) {
            (
                PatternLiteral::Int {
                    value: start_value,
                    negative: start_negative,
                    ..
                },
                PatternLiteral::Int {
                    value: end_value,
                    negative: end_negative,
                    ..
                },
            ) => match kind {
                TyKind::Int(_) => {
                    let start = signed_value(start_negative, start_value);
                    let end = signed_value(end_negative, end_value);
                    if inclusive { start > end } else { start >= end }
                }
                TyKind::Uint(_) => {
                    if start_negative || end_negative {
                        false
                    } else if inclusive {
                        start_value > end_value
                    } else {
                        start_value >= end_value
                    }
                }
                _ => false,
            },
            (PatternLiteral::Char(start), PatternLiteral::Char(end)) => {
                if inclusive {
                    start > end
                } else {
                    start >= end
                }
            }
            _ => false,
        };
        if empty {
            self.error(codes::E2050, span, "this range pattern is empty")
                .help("swap the endpoints, or remove the arm")
                .emit();
        }
    }

    fn check_range_bound(&mut self, literal: PatternLiteral, expected: TyId, span: Span) {
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
            PatternKind::Binding { def, pattern, .. } => {
                let ty = self.local_type(*def);
                if !self.is_copy(ty) {
                    out.insert(*def);
                }
                if let Some(pattern) = pattern {
                    self.collect_noncopy_bindings(*pattern, out);
                }
            }
            PatternKind::Variant { fields, .. } | PatternKind::Struct { fields, .. } => {
                for (_, field) in fields {
                    self.collect_noncopy_bindings(*field, out);
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements {
                    self.collect_noncopy_bindings(*element, out);
                }
            }
            PatternKind::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix.iter()) {
                    self.collect_noncopy_bindings(*element, out);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.collect_noncopy_bindings(*alternative, out);
                }
            }
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Range { .. }
            | PatternKind::Error => {}
        }
    }
}

fn signed_value(negative: bool, magnitude: u128) -> i128 {
    let magnitude = magnitude.min(i128::MAX as u128) as i128;
    if negative { -magnitude } else { magnitude }
}
