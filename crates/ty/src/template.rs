//! Structural matching and unification of type templates.
//!
//! A *template* is a type that may contain declared parameters. Two templates
//! unify when there is a substitution for their parameters that makes them
//! equal. This is different from inference: parameters are rigid inside a body,
//! but two declaration headers are compared by asking whether a common instance
//! exists. Coherence, inherent-impl overlap, and impl selection all need this
//! question, so the algorithm lives next to the type representation instead of
//! in each consumer.

use seal_hash::FxHashMap;

use crate::ctxt::TyCtxt;
use crate::id::{ParamTyId, TyId};
use crate::kind::TyKind;

/// The substitutions that make two templates equal.
///
/// Keys are parameters of the left and right templates respectively. Values are
/// resolved neither deeply nor eagerly: a binding may mention a parameter of the
/// other side, which keeps the algorithm linear.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateMatch {
    left: FxHashMap<ParamTyId, TyId>,
    right: FxHashMap<ParamTyId, TyId>,
}

impl TemplateMatch {
    /// Returns the substitution discovered for the left template's parameters.
    pub fn left(&self) -> &FxHashMap<ParamTyId, TyId> {
        &self.left
    }

    /// Returns the substitution discovered for the right template's parameters.
    pub fn right(&self) -> &FxHashMap<ParamTyId, TyId> {
        &self.right
    }
}

/// Matches `pattern` against the rigid `actual`, binding pattern parameters.
///
/// A parameter may appear several times; every occurrence must receive the same
/// type. This is the same rule as unification, minus inference variables: an
/// unresolved inference variable on the `actual` side makes the match fail,
/// because no decision can be made from it.
///
/// Identical templates that contain parameters still record one self-binding per
/// parameter, so callers always receive a complete substitution for the pattern.
pub fn match_template(
    ctxt: &TyCtxt,
    pattern: TyId,
    actual: TyId,
    mapping: &mut FxHashMap<ParamTyId, TyId>,
) -> bool {
    if pattern == actual && !ctxt.contains_param(pattern) {
        return true;
    }
    match (ctxt.kind(pattern), ctxt.kind(actual)) {
        (TyKind::Error, _) | (_, TyKind::Error) => true,
        (TyKind::Param(param), _) => match mapping.get(&param).copied() {
            Some(previous) => previous == actual,
            None => {
                if matches!(ctxt.kind(actual), TyKind::Infer(_)) {
                    return false;
                }
                mapping.insert(param, actual);
                true
            }
        },
        (
            TyKind::Ref {
                kind: pattern_kind,
                pointee: pattern_pointee,
            },
            TyKind::Ref {
                kind: actual_kind,
                pointee: actual_pointee,
            },
        ) if pattern_kind == actual_kind => {
            match_template(ctxt, pattern_pointee, actual_pointee, mapping)
        }
        (
            TyKind::RawPtr {
                kind: pattern_kind,
                pointee: pattern_pointee,
            },
            TyKind::RawPtr {
                kind: actual_kind,
                pointee: actual_pointee,
            },
        ) if pattern_kind == actual_kind => {
            match_template(ctxt, pattern_pointee, actual_pointee, mapping)
        }
        (
            TyKind::Array {
                element: pattern_element,
                length: pattern_length,
            },
            TyKind::Array {
                element: actual_element,
                length: actual_length,
            },
        ) if pattern_length == actual_length => {
            match_template(ctxt, pattern_element, actual_element, mapping)
        }
        (
            TyKind::Slice {
                element: pattern_element,
            },
            TyKind::Slice {
                element: actual_element,
            },
        ) => match_template(ctxt, pattern_element, actual_element, mapping),
        (TyKind::Tuple(pattern_tuple), TyKind::Tuple(actual_tuple)) => {
            let patterns = ctxt.tuple_elements(pattern_tuple);
            let actuals = ctxt.tuple_elements(actual_tuple);
            patterns.len() == actuals.len()
                && patterns
                    .into_iter()
                    .zip(actuals)
                    .all(|(pattern, actual)| match_template(ctxt, pattern, actual, mapping))
        }
        (
            TyKind::Adt {
                adt: pattern_adt,
                args: pattern_args,
            },
            TyKind::Adt {
                adt: actual_adt,
                args: actual_args,
            },
        ) if pattern_adt == actual_adt => {
            let patterns = ctxt.substs_args(pattern_args);
            let actuals = ctxt.substs_args(actual_args);
            patterns.len() == actuals.len()
                && patterns
                    .into_iter()
                    .zip(actuals)
                    .all(|(pattern, actual)| match_template(ctxt, pattern, actual, mapping))
        }
        (
            TyKind::Dyn {
                trait_def: pattern_trait,
                args: pattern_args,
            },
            TyKind::Dyn {
                trait_def: actual_trait,
                args: actual_args,
            },
        ) if pattern_trait == actual_trait => {
            let patterns = ctxt.substs_args(pattern_args);
            let actuals = ctxt.substs_args(actual_args);
            patterns.len() == actuals.len()
                && patterns
                    .into_iter()
                    .zip(actuals)
                    .all(|(pattern, actual)| match_template(ctxt, pattern, actual, mapping))
        }
        _ => false,
    }
}

/// Unifies two templates, allowing parameters on both sides to be bound.
///
/// Returns the substitutions that make the templates equal, or `None` when no
/// instance can satisfy both. The two sides must declare disjoint parameters,
/// which holds when they come from different impl blocks.
pub fn unify_templates(ctxt: &TyCtxt, left: TyId, right: TyId) -> Option<TemplateMatch> {
    let mut unifier = TemplateUnifier::new(ctxt);
    unifier.unify(left, right).then(|| unifier.into_match())
}

/// Incremental template unification, for headers with several parts.
///
/// An impl header is a self type plus trait arguments. Checking them with one
/// unifier keeps a parameter that appears in both parts consistent.
pub struct TemplateUnifier<'a> {
    ctxt: &'a TyCtxt,
    matched: TemplateMatch,
}

impl<'a> TemplateUnifier<'a> {
    /// Creates an empty unifier over `ctxt`.
    pub fn new(ctxt: &'a TyCtxt) -> Self {
        Self {
            ctxt,
            matched: TemplateMatch::default(),
        }
    }

    /// Adds one equality constraint, returning `false` when it cannot hold.
    pub fn unify(&mut self, left: TyId, right: TyId) -> bool {
        self.unify_inner(left, right)
    }

    /// Returns the accumulated substitutions.
    pub fn into_match(self) -> TemplateMatch {
        self.matched
    }
}

impl TemplateUnifier<'_> {
    fn resolve(&self, ty: TyId, left: bool) -> TyId {
        let map = if left {
            &self.matched.left
        } else {
            &self.matched.right
        };
        let mut current = ty;
        let mut steps = 0usize;
        while let TyKind::Param(param) = self.ctxt.kind(current) {
            let Some(next) = map.get(&param).copied() else {
                break;
            };
            if next == current || steps > 64 {
                break;
            }
            current = next;
            steps += 1;
        }
        current
    }

    fn unify_inner(&mut self, left: TyId, right: TyId) -> bool {
        let left = self.resolve(left, true);
        let right = self.resolve(right, false);
        if left == right {
            return true;
        }
        match (self.ctxt.kind(left), self.ctxt.kind(right)) {
            (TyKind::Error, _) | (_, TyKind::Error) => true,
            (TyKind::Param(left_param), TyKind::Param(_)) => {
                self.matched.left.insert(left_param, right);
                true
            }
            (TyKind::Param(left_param), _) => {
                self.matched.left.insert(left_param, right);
                true
            }
            (_, TyKind::Param(right_param)) => {
                self.matched.right.insert(right_param, left);
                true
            }
            (
                TyKind::Ref {
                    kind: left_kind,
                    pointee: left_pointee,
                },
                TyKind::Ref {
                    kind: right_kind,
                    pointee: right_pointee,
                },
            ) if left_kind == right_kind => self.unify(left_pointee, right_pointee),
            (
                TyKind::RawPtr {
                    kind: left_kind,
                    pointee: left_pointee,
                },
                TyKind::RawPtr {
                    kind: right_kind,
                    pointee: right_pointee,
                },
            ) if left_kind == right_kind => self.unify(left_pointee, right_pointee),
            (
                TyKind::Array {
                    element: left_element,
                    length: left_length,
                },
                TyKind::Array {
                    element: right_element,
                    length: right_length,
                },
            ) if left_length == right_length => self.unify(left_element, right_element),
            (
                TyKind::Slice {
                    element: left_element,
                },
                TyKind::Slice {
                    element: right_element,
                },
            ) => self.unify(left_element, right_element),
            (TyKind::Tuple(left_tuple), TyKind::Tuple(right_tuple)) => {
                let lefts = self.ctxt.tuple_elements(left_tuple);
                let rights = self.ctxt.tuple_elements(right_tuple);
                lefts.len() == rights.len()
                    && lefts
                        .into_iter()
                        .zip(rights)
                        .all(|(left, right)| self.unify(left, right))
            }
            (
                TyKind::Adt {
                    adt: left_adt,
                    args: left_args,
                },
                TyKind::Adt {
                    adt: right_adt,
                    args: right_args,
                },
            ) if left_adt == right_adt => {
                let lefts = self.ctxt.substs_args(left_args);
                let rights = self.ctxt.substs_args(right_args);
                lefts.len() == rights.len()
                    && lefts
                        .into_iter()
                        .zip(rights)
                        .all(|(left, right)| self.unify(left, right))
            }
            (
                TyKind::Dyn {
                    trait_def: left_trait,
                    args: left_args,
                },
                TyKind::Dyn {
                    trait_def: right_trait,
                    args: right_args,
                },
            ) if left_trait == right_trait => {
                let lefts = self.ctxt.substs_args(left_args);
                let rights = self.ctxt.substs_args(right_args);
                lefts.len() == rights.len()
                    && lefts
                        .into_iter()
                        .zip(rights)
                        .all(|(left, right)| self.unify(left, right))
            }
            _ => false,
        }
    }
}
