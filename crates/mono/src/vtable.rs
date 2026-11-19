//! Vtable collection for specialized trait-object coercions.
//!
//! One vtable exists per distinct concrete pointee and principal trait
//! reference. The concrete type lives in [`VtableKey::concrete`] and also in
//! the key's trait reference as `self_ty`, so layout sees the real implementor
//! rather than the `dyn` type.
//!
//! Method entries follow [`VtableLayout`] order: the principal trait's methods,
//! then each supertrait depth-first, with every declaration keeping one slot.
//! Each entry is the concrete implementation selected by the existing trait
//! instance resolver.

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_layout::VtableLayout;
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{Instance, TraitRef, TyCtxt, TyId};

use crate::error::MonoError;
use crate::substitute::resolve_trait_instance;

/// The identity of one vtable.
///
/// `concrete` is the implementing pointee, never the `dyn` type. `trait_ref`
/// uses that same concrete type as `self_ty` and keeps the instantiated trait
/// arguments from the coercion target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VtableKey {
    /// The concrete pointee selected by the coercion source.
    pub concrete: TyId,
    /// The principal trait reference with `self_ty == concrete`.
    pub trait_ref: TraitRef,
}

impl VtableKey {
    /// Derives a vtable key from a trait-object coercion.
    ///
    /// Code generation uses this to resolve the exact table emitted for a
    /// `MakeTraitObject` rvalue. Returns `None` for malformed pointer or
    /// trait-object shapes; MIR validation rejects those before lowering.
    pub fn from_coercion(tcx: &TyCtxt, source_ty: TyId, target_ty: TyId) -> Option<Self> {
        let (_, concrete) = tcx.ref_parts(source_ty)?;
        let (_, object) = tcx.ref_parts(target_ty)?;
        let (trait_def, args) = tcx.dyn_parts(object)?;
        Some(Self {
            concrete,
            trait_ref: TraitRef::new(trait_def, concrete, args),
        })
    }
}

/// One collected vtable.
///
/// The first emitted slot is the drop glue of [`VtableKey::concrete`], which
/// code generation resolves from the key and the collected glue rather than
/// storing a second copy of the same instance here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonoVTable {
    /// The concrete type and principal trait this vtable belongs to.
    pub key: VtableKey,
    /// Concrete method instances in [`VtableLayout`] slot order.
    ///
    /// Header entries are not instances; `entries[0]` is the first method
    /// after the three-entry header.
    pub entries: Vec<Instance>,
}

/// Derives a vtable key from a trait-object coercion.
///
/// Returns `None` when either side is not a pointer or the target pointee is
/// not a trait object. MIR validation rejects those shapes before
/// monomorphization; this is a defensive check.
pub(crate) fn key_of(tcx: &TyCtxt, source_ty: TyId, target_ty: TyId) -> Option<VtableKey> {
    VtableKey::from_coercion(tcx, source_ty, target_ty)
}

/// Resolves every method entry for `key`.
///
/// The layout is computed from the instantiated principal trait. Each method
/// declaration becomes an instance with `[concrete, declaring trait args]` and
/// is resolved through the same implementation selection used by static trait
/// calls, so a missing or ambiguous impl is a typed error rather than a panic.
pub(crate) fn build_vtable(
    hir: &Hir,
    interner: &Interner,
    key: VtableKey,
) -> Result<MonoVTable, MonoError> {
    let tcx = hir.ty_ctxt();
    let layout =
        VtableLayout::new(tcx, key.trait_ref).map_err(|error| MonoError::VtableLayout {
            detail: error.to_string(),
        })?;
    let mut entries = Vec::with_capacity(layout.method_count());
    for entry in layout.method_entries() {
        let method = entry.method().ok_or_else(|| MonoError::VtableLayout {
            detail: "a method entry has no declaration".to_owned(),
        })?;
        let declaring_trait = entry
            .declaring_trait()
            .ok_or_else(|| MonoError::VtableLayout {
                detail: "a method entry has no declaring trait".to_owned(),
            })?;
        let mut arguments = Vec::with_capacity(1 + tcx.substs_len(declaring_trait.args));
        arguments.push(key.concrete);
        arguments.extend(tcx.substs_args(declaring_trait.args));
        let instance = Instance::new(method, tcx.substs(&arguments));
        entries.push(resolve_trait_instance(hir, interner, instance)?);
    }
    Ok(MonoVTable { key, entries })
}

/// Reports a vtable failure as the user-facing missing-entry diagnostic.
pub(crate) fn report(dcx: &DiagCtxt, span: Span, error: &MonoError) {
    dcx.error(error.to_string())
        .code(codes::E7005.diag_code())
        .primary(span, "this trait-object coercion needs a complete vtable")
        .emit();
}
