//! Type queries and display used by the checker.

use seal_ast::PrimitiveType;
use seal_hir::primitive_kind;
use seal_ty::{TyId, TyKind};

use super::Checker;

impl Checker<'_> {
    pub(super) fn interned(&self, kind: TyKind) -> TyId {
        self.hir.ty_ctxt().lookup(kind).unwrap_or_else(|| {
            panic!(
                "type `{kind:?}` was not interned during HIR lowering; \
                 this is an internal compiler error"
            )
        })
    }

    pub(super) fn unit(&self) -> TyId {
        self.interned(TyKind::Unit)
    }

    pub(super) fn bool_ty(&self) -> TyId {
        self.interned(TyKind::Bool)
    }

    pub(super) fn i32_ty(&self) -> TyId {
        self.interned(primitive_kind(PrimitiveType::I32))
    }

    pub(super) fn f64_ty(&self) -> TyId {
        self.interned(primitive_kind(PrimitiveType::F64))
    }

    pub(super) fn error_ty(&self) -> TyId {
        self.interned(TyKind::Error)
    }

    pub(super) fn never(&self) -> TyId {
        self.interned(TyKind::Never)
    }

    pub(super) fn str_ref_ty(&self) -> TyId {
        let str_ty = self.interned(TyKind::Str);
        self.interned(TyKind::Ref(str_ty))
    }

    /// Returns the shared reference type `*pointee`, interning it if needed.
    ///
    /// Borrowing a place creates a type the source never wrote, so this cannot
    /// go through [`Checker::interned`], which only looks up existing kinds.
    pub(super) fn ref_ty(&self, pointee: TyId) -> TyId {
        self.hir.ty_ctxt().intern(TyKind::Ref(pointee))
    }

    pub(super) fn ty_text(&self, ty: TyId) -> String {
        self.hir.ty_ctxt().display(ty).to_string()
    }

    pub(super) fn is_error(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Error)
    }

    pub(super) fn is_never(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Never)
    }

    pub(super) fn is_bool(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Bool)
    }

    pub(super) fn is_unit(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Unit)
    }

    pub(super) fn is_char(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Char)
    }

    pub(super) fn is_float(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), TyKind::Float(_))
    }

    pub(super) fn is_integer(&self, ty: TyId) -> bool {
        matches!(
            self.hir.ty_ctxt().kind(ty),
            TyKind::Int(_) | TyKind::Uint(_)
        )
    }

    pub(super) fn is_numeric(&self, ty: TyId) -> bool {
        matches!(
            self.hir.ty_ctxt().kind(ty),
            TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_)
        )
    }

    pub(super) fn is_sized(&self, ty: TyId) -> bool {
        self.hir.ty_ctxt().is_sized(ty)
    }

    /// Returns the bit width of an integer type, resolving `isize`/`usize` from
    /// the target's pointer width.
    pub(super) fn integer_bits(&self, ty: TyId) -> Option<u16> {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Int(int) => Some(int.bits().unwrap_or(self.pointer_bits())),
            TyKind::Uint(uint) => Some(uint.bits().unwrap_or(self.pointer_bits())),
            _ => None,
        }
    }

    pub(super) fn pointer_bits(&self) -> u16 {
        u16::from(self.pointer_width)
    }

    /// Returns `true` when `found` satisfies `expected`.
    ///
    /// `Error` absorbs anything: its diagnostic was already reported. `Never`
    /// fits every type because the expression cannot produce a value.
    pub(super) fn is_compatible(&self, expected: TyId, found: TyId) -> bool {
        expected == found || self.is_error(expected) || self.is_error(found) || self.is_never(found)
    }

    /// Resolves a declared type for storage, reporting unsized types.
    ///
    /// The returned type is the error type when the declaration is unsized, so
    /// later checks do not cascade from a value that cannot be stored.
    pub(super) fn storage_ty(&self, ty: TyId, span: seal_span::Span, what: &str) -> TyId {
        if self.is_error(ty) || self.is_sized(ty) {
            return ty;
        }
        self.report_unsized(span, ty, what);
        self.error_ty()
    }
}
