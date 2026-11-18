//! Checking reference arguments at call sites.
//!
//! Passing a reference to a call lets the callee access its pointee. An
//! exclusive argument (`*mut T`, or an aggregate carrying one) is stronger: the
//! callee gets sole access for the whole call. The checker therefore treats such
//! an argument as an access to the pointee and requires:
//!
//! - no live loan of the pointee besides the loans that authorize the argument;
//! - no other reference argument may name the same storage.
//!
//! Both rules compare storage with the may-name analysis, so copies and
//! reborrows are ordered. When a reference argument's target is unknown, the
//! checker cannot prove the contract: it stays silent, and
//! [`crate::aliasing`] withholds `noalias` for the callee instead.

use seal_hir::Hir;
use seal_mir::{Body, Operand, Place};
use seal_span::Span;

use crate::aliasing::ParamKind;
use crate::holds::{HoldState, LoanSet};
use crate::id::LoanId;
use crate::loan::Loan;
use crate::origins::{self, OriginSet, OriginState};
use crate::report::Reporter;

/// One call under check, with the analyses at that program point.
pub(crate) struct CallSite<'a> {
    /// The arguments in parameter order.
    pub(crate) args: &'a [Operand],
    /// The source span of each argument, matching `args` by position.
    pub(crate) arg_spans: &'a [Span],
    /// The span of the call statement.
    pub(crate) span: Span,
    /// The may-name state before the call.
    pub(crate) origins: &'a OriginState,
    /// The may-hold state before the call.
    pub(crate) holds: &'a HoldState,
    /// The loan table.
    pub(crate) loans: &'a [Loan],
    /// The loans active at the call.
    pub(crate) active: &'a LoanSet,
    /// Locals holding a materialized constant value.
    pub(crate) const_locals: &'a origins::ConstLocals,
    /// HIR, used for place types.
    pub(crate) hir: &'a Hir,
    /// The body being checked.
    pub(crate) body: &'a Body,
    /// Diagnostic builder.
    pub(crate) reporter: &'a Reporter<'a>,
}

/// What checking one call proved.
pub(crate) struct Outcome {
    /// Every reference argument has known storage and exclusive arguments do
    /// not alias another argument.
    pub(crate) proven: bool,
    /// Some argument reached storage through a reference of unknown target.
    pub(crate) unknown: bool,
}

/// One argument's reference facts.
struct Argument {
    place: Place,
    /// The argument's own source span, so the diagnostic points at it.
    span: Span,
    /// Position in the source argument list, counted from one for people.
    position: usize,
    origins: Option<OriginSet>,
    kind: ParamKind,
}

/// Checks the arguments of one call.
pub(crate) fn check(site: CallSite<'_>) -> Outcome {
    let mut proven = true;
    let mut unknown = false;
    let mut arguments = Vec::with_capacity(site.args.len());

    for (position, argument) in site.args.iter().enumerate() {
        let Some(place) = argument.place() else {
            continue;
        };
        // A rejected borrow has already been explained; checking arguments
        // derived from it would only repeat that diagnostic.
        if site
            .holds
            .at(&place)
            .iter()
            .any(|id| site.loans[id.index() as usize].reported.get())
        {
            proven = false;
            continue;
        }
        // A scalar argument reached through a dereference is still an access.
        if origins::keys_of(site.origins, &place, site.const_locals).is_unknown() {
            unknown = true;
            proven = false;
        }
        let ty = origins::place_ty(site.hir, site.body, &place);
        let kind = ParamKind::of(site.hir.ty_ctxt(), ty);
        if !kind.has_reference {
            continue;
        }
        // The argument's value is what the callee receives. A reference loaded
        // out of memory has no tracked target, even though reading the place
        // itself touches storage the analysis may know.
        let set = if place.is_deref() {
            OriginSet::unknown()
        } else {
            site.origins.at(&place)
        };
        let origins = if set.is_empty() || set.is_unknown() {
            unknown = true;
            proven = false;
            None
        } else {
            Some(set)
        };
        let span = site.arg_spans.get(position).copied().unwrap_or(site.span);
        arguments.push(Argument {
            place,
            span,
            position,
            origins,
            kind,
        });
    }

    for argument in &arguments {
        if !argument.kind.may_write {
            continue;
        }
        let Some(origins) = &argument.origins else {
            continue;
        };
        for id in conflicting_loans(&site, origins, &argument.place) {
            site.reporter.exclusive_argument_borrowed(
                &argument.place,
                argument.span,
                &site.loans[id.index() as usize],
            );
        }
    }

    for right in 0..arguments.len() {
        for left in 0..right {
            let left_arg = &arguments[left];
            let right_arg = &arguments[right];
            if !(left_arg.kind.may_write || right_arg.kind.may_write) {
                continue;
            }
            let (Some(left_origins), Some(right_origins)) = (&left_arg.origins, &right_arg.origins)
            else {
                continue;
            };
            if origins::keys_overlap(left_origins, right_origins) {
                site.reporter.arguments_alias(
                    site.span,
                    (&left_arg.place, left_arg.span, left_arg.position),
                    (&right_arg.place, right_arg.span, right_arg.position),
                );
            }
        }
    }

    Outcome { proven, unknown }
}

/// Returns the live loans of `origins` that the argument does not itself carry.
///
/// The loans held by the argument are the ones that authorize it, and their
/// ancestors authorized those in turn, so none of them conflict with the call.
fn conflicting_loans(site: &CallSite<'_>, origins: &OriginSet, argument: &Place) -> Vec<LoanId> {
    let mut excluded = LoanSet::default();
    for id in site.holds.at(argument) {
        if origins::keys_overlap(origins, &site.loans[id.index() as usize].keys) {
            excluded.insert(id);
        }
    }
    crate::loan::expand_parents(site.loans, &mut excluded);
    let mut ids: Vec<LoanId> = site
        .active
        .iter()
        .copied()
        .filter(|id| {
            !excluded.contains(id)
                && !site.loans[id.index() as usize].reported.get()
                && origins::keys_overlap(origins, &site.loans[id.index() as usize].keys)
        })
        .collect();
    ids.sort_by_key(|id| id.index());
    ids
}
