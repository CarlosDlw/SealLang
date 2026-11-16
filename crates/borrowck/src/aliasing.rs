//! Aliasing facts behind the exclusive-reference contract.
//!
//! A `*mut T` parameter grants the callee sole access to its pointee (D6). LLVM
//! may only assume that grant when the whole program upholds it, so these facts
//! are computed after the body walk and gate the `noalias` attribute:
//!
//! - The callee must never reach storage through a reference whose target is
//!   unknown: a pointer loaded out of memory could name the pointee through a
//!   pointer the analysis cannot order.
//! - Every call site must prove that its reference arguments have known storage
//!   and that no exclusive argument may alias another argument.
//!
//! Emitting `noalias` is an optimization, not a language rule. When a proof does
//! not close, the parameter keeps its ordinary ABI.

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_mir::Mir;
use seal_ty::{RefKind, TyCtxt, TyKind};

/// What a parameter allows the callee to do through its references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParamKind {
    /// The parameter type contains a reference at any depth.
    pub(crate) has_reference: bool,
    /// The parameter type contains an exclusive reference at any depth, so the
    /// callee may write through it.
    pub(crate) may_write: bool,
}

impl ParamKind {
    /// Classifies a parameter or argument type.
    pub(crate) fn of(tcx: &TyCtxt, ty: seal_ty::TyId) -> Self {
        match tcx.kind(ty) {
            TyKind::Ref { kind, .. } => Self {
                has_reference: true,
                may_write: kind == RefKind::Exclusive,
            },
            _ => Self {
                has_reference: tcx.contains_reference(ty),
                may_write: tcx.contains_exclusive_reference(ty),
            },
        }
    }
}

/// Whether every call site of a function proved the exclusive contract.
#[derive(Default)]
pub(crate) struct CallSites(FxHashMap<DefId, bool>);

impl CallSites {
    /// Records one call, keeping only whether all of them proved the contract.
    pub(crate) fn record(&mut self, callee: DefId, proven: bool) {
        self.0
            .entry(callee)
            .and_modify(|proven_all| *proven_all &= proven)
            .or_insert(proven);
    }

    /// Returns whether every known call site of `callee` proved the contract.
    ///
    /// A function no body calls is vacuously proven: no call can break its
    /// assumptions.
    fn proven(&self, callee: DefId) -> bool {
        self.0.get(&callee).copied().unwrap_or(true)
    }
}

/// Per-body facts that decide whether its exclusive parameters may be `noalias`.
pub(crate) struct BodyFacts {
    /// The body reached storage through a reference with an unknown target.
    pub(crate) unknown: bool,
}

/// The functions whose exclusive-reference parameters the backend may mark
/// `noalias`.
#[derive(Clone, Debug, Default)]
pub struct AliasingFacts {
    noalias: FxHashSet<DefId>,
}

impl AliasingFacts {
    /// Returns the functions whose exclusive-reference parameters are proven
    /// usable without aliasing.
    ///
    /// The set is a conservative proof, never a language guarantee: a function
    /// outside it may still be correct, it just does not justify the attribute.
    #[inline]
    pub fn noalias_functions(&self) -> &FxHashSet<DefId> {
        &self.noalias
    }
}

/// Builds the `noalias` facts from the checked bodies and their call sites.
pub(crate) fn facts(
    tcx: &TyCtxt,
    mir: &Mir,
    bodies: &FxHashMap<DefId, BodyFacts>,
    calls: &CallSites,
) -> AliasingFacts {
    let mut noalias = FxHashSet::default();
    for body in mir.bodies() {
        let has_exclusive = body.args().any(|argument| {
            matches!(
                tcx.kind(argument.ty),
                TyKind::Ref {
                    kind: RefKind::Exclusive,
                    ..
                }
            )
        });
        if !has_exclusive {
            continue;
        }
        let clean = bodies.get(&body.owner).is_some_and(|facts| !facts.unknown);
        if clean && calls.proven(body.owner) {
            noalias.insert(body.owner);
        }
    }
    AliasingFacts { noalias }
}
