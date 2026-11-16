//! Types computed by the checker.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::HirExprId;
use seal_ty::{PathElem, RefKind, TyId};

/// One step from a place to a sub-place, with the field resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceProjection {
    /// Reads through a shared reference.
    Deref,
    /// Selects a struct field by its definition.
    Field(DefId),
    /// Selects an array element.
    Index {
        /// The index expression, evaluated by MIR lowering.
        index: HirExprId,
        /// The constant value when the index is a literal in range.
        constant: Option<u64>,
        /// The array length, used for the runtime bounds check.
        length: u64,
    },
}

/// A place resolved by type checking.
///
/// HIR keeps field names symbolic because resolving them needs the base type.
/// The checker records the resolved form once, and later phases (MIR lowering,
/// move analysis) consume it instead of resolving again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPlace {
    /// The local or parameter the place starts from.
    pub local: DefId,
    /// The declared type of that local.
    pub root_ty: TyId,
    /// The projections in application order.
    pub projections: Vec<PlaceProjection>,
    /// The type of the selected place.
    pub ty: TyId,
    /// The kind of the last dereference among the projections, or `None` when
    /// no projection reads through a reference.
    ///
    /// Writing a place through a dereference is allowed only for
    /// [`RefKind::Exclusive`]; the last dereference decides, because it is the
    /// reference that reaches the selected storage.
    pub deref: Option<RefKind>,
}

impl ResolvedPlace {
    /// Returns `true` if any projection reads through a reference.
    pub fn contains_deref(&self) -> bool {
        self.projections.contains(&PlaceProjection::Deref)
    }

    /// Returns `true` if the place names the whole local.
    pub fn is_whole(&self) -> bool {
        self.projections.is_empty()
    }

    /// Returns the move path selected by the place.
    ///
    /// Dereferences do not contribute: initialization is only tracked inside one
    /// frame's storage. A dynamic index stops the path at the array it selects,
    /// because its element is not known at compile time; the whole array then
    /// covers the access conservatively.
    pub fn path(&self) -> Vec<PathElem> {
        let mut path = Vec::new();
        for projection in &self.projections {
            match projection {
                PlaceProjection::Deref => break,
                PlaceProjection::Field(field) => path.push(PathElem::Field(*field)),
                PlaceProjection::Index {
                    constant: Some(index),
                    ..
                } => path.push(PathElem::Index(*index)),
                PlaceProjection::Index { constant: None, .. } => break,
            }
        }
        path
    }

    /// Returns `true` when the place indexes an array with a runtime value.
    pub fn has_dynamic_index(&self) -> bool {
        self.projections
            .iter()
            .any(|projection| matches!(projection, PlaceProjection::Index { constant: None, .. }))
    }
}

/// A function signature as seen by callers.
///
/// Parameters whose declared type is unsized are normalized to the error type,
/// because such a declaration already produced `E3010`. The return type is the
/// unit type when the declaration omits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// Parameter types in source order.
    pub params: Vec<TyId>,
    /// The return type.
    pub ret: TyId,
}

/// Type information for one file.
///
/// Maps are keyed by HIR IDs, so no lookup re-resolves a name or re-reads the
/// source. The callee of a valid call has no entry: Seal has no first-class
/// function type, and the callee is resolved directly to its function
/// definition.
#[derive(Clone, Debug)]
pub struct TypeckResults {
    expr_ty: FxHashMap<HirExprId, TyId>,
    local_ty: FxHashMap<DefId, TyId>,
    signatures: FxHashMap<DefId, Signature>,
    places: FxHashMap<HirExprId, ResolvedPlace>,
}

impl TypeckResults {
    /// Returns the type of `expr`, if it is a value expression.
    pub fn expr_ty(&self, expr: HirExprId) -> Option<TyId> {
        self.expr_ty.get(&expr).copied()
    }

    /// Returns the type of a parameter or binding.
    pub fn local_ty(&self, local: DefId) -> Option<TyId> {
        self.local_ty.get(&local).copied()
    }

    /// Returns the signature of a function.
    pub fn signature(&self, function: DefId) -> Option<&Signature> {
        self.signatures.get(&function)
    }

    /// Returns the resolved place denoted by `expr`, if it denotes storage.
    pub fn place(&self, expr: HirExprId) -> Option<&ResolvedPlace> {
        self.places.get(&expr)
    }

    /// Returns the number of expressions that received a type.
    pub fn typed_expr_count(&self) -> usize {
        self.expr_ty.len()
    }

    pub(crate) fn new() -> Self {
        Self {
            expr_ty: FxHashMap::default(),
            local_ty: FxHashMap::default(),
            signatures: FxHashMap::default(),
            places: FxHashMap::default(),
        }
    }

    pub(crate) fn record_expr(&mut self, expr: HirExprId, ty: TyId) {
        self.expr_ty.insert(expr, ty);
    }

    pub(crate) fn record_local(&mut self, local: DefId, ty: TyId) {
        self.local_ty.insert(local, ty);
    }

    pub(crate) fn record_place(&mut self, expr: HirExprId, place: ResolvedPlace) {
        self.places.insert(expr, place);
    }

    pub(crate) fn insert_signature(&mut self, function: DefId, signature: Signature) {
        self.signatures.insert(function, signature);
    }
}
