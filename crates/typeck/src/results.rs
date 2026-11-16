//! Types computed by the checker.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::HirExprId;
use seal_ty::TyId;

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

    /// Returns the number of expressions that received a type.
    pub fn typed_expr_count(&self) -> usize {
        self.expr_ty.len()
    }

    pub(crate) fn new() -> Self {
        Self {
            expr_ty: FxHashMap::default(),
            local_ty: FxHashMap::default(),
            signatures: FxHashMap::default(),
        }
    }

    pub(crate) fn record_expr(&mut self, expr: HirExprId, ty: TyId) {
        self.expr_ty.insert(expr, ty);
    }

    pub(crate) fn record_local(&mut self, local: DefId, ty: TyId) {
        self.local_ty.insert(local, ty);
    }

    pub(crate) fn insert_signature(&mut self, function: DefId, signature: Signature) {
        self.signatures.insert(function, signature);
    }
}
