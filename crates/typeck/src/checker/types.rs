//! Type queries and display used by the checker.

use seal_ast::PrimitiveType;
use seal_hir::primitive_kind;
use seal_layout::{LayoutCtxt, LayoutError};
use seal_ty::{RefKind, TyId, TyKind};

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

    pub(super) fn u32_ty(&self) -> TyId {
        self.interned(primitive_kind(PrimitiveType::U32))
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

    pub(super) fn usize_ty(&self) -> TyId {
        self.interned(primitive_kind(PrimitiveType::Usize))
    }

    pub(super) fn array_ty(&self, element: TyId, length: u64) -> TyId {
        self.hir.ty_ctxt().array_ty(element, length)
    }

    pub(super) fn array_parts(&self, ty: TyId) -> Option<(TyId, u64)> {
        self.hir.ty_ctxt().array_parts(ty)
    }

    pub(super) fn slice_ty(&self, element: TyId) -> TyId {
        self.hir.ty_ctxt().slice_ty(element)
    }

    pub(super) fn slice_parts(&self, ty: TyId) -> Option<TyId> {
        self.hir.ty_ctxt().slice_parts(ty)
    }

    pub(super) fn str_ref_ty(&self) -> TyId {
        let str_ty = self.interned(TyKind::Str);
        self.interned(TyKind::Ref {
            kind: RefKind::Shared,
            pointee: str_ty,
        })
    }

    /// Returns the reference type `kind` to `pointee`, interning it if needed.
    ///
    /// Borrowing a place creates a type the source never wrote, so this cannot
    /// go through [`Checker::interned`], which only looks up existing kinds.
    pub(super) fn ref_ty(&self, kind: RefKind, pointee: TyId) -> TyId {
        self.hir.ty_ctxt().ref_ty_with(kind, pointee)
    }

    pub(super) fn ty_text(&self, ty: TyId) -> String {
        self.hir
            .ty_ctxt()
            .display_with(ty, self.interner)
            .to_string()
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

    pub(super) fn is_copy(&self, ty: TyId) -> bool {
        self.hir.ty_ctxt().is_copy(ty)
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

    /// Resolves a declared type for storage, reporting unsized types and
    /// arrays that cannot fit in the target's address space.
    ///
    /// The returned type is the error type when the declaration is invalid, so
    /// later checks do not cascade from a value that cannot be stored.
    pub(super) fn storage_ty(&self, ty: TyId, span: seal_span::Span, what: &str) -> TyId {
        if self.is_error(ty) {
            return ty;
        }
        if !self.is_sized(ty) {
            self.report_unsized(span, ty, what);
            return self.error_ty();
        }
        if self.array_exceeds_address_space(ty) {
            self.report_array_too_large(span, ty, what);
            return self.error_ty();
        }
        ty
    }

    /// Returns `true` when laying `ty` out overflows or exceeds what a target
    /// pointer can address.
    ///
    /// Recursive and unsized types are reported by other checks, so only the
    /// size failures belong to `E3023` here.
    pub(super) fn array_exceeds_address_space(&self, ty: TyId) -> bool {
        let layout = match LayoutCtxt::new(self.hir.ty_ctxt(), self.pointer_width).layout_of(ty) {
            Ok(layout) => layout,
            Err(LayoutError::TooLarge(_)) => return true,
            Err(_) => return false,
        };
        let address_space_max = if self.pointer_width >= 64 {
            u64::MAX
        } else {
            (1u64 << self.pointer_width) - 1
        };
        layout.size() > address_space_max
    }

    pub(super) fn report_array_too_large(&self, span: seal_span::Span, ty: TyId, what: &str) {
        self.error(
            seal_error_codes::E3023,
            span,
            format!(
                "the type `{}` is too large for the {}-bit target",
                self.ty_text(ty),
                self.pointer_width
            ),
        )
        .help(format!(
            "use a smaller array length or element type for this {what}"
        ))
        .emit();
    }
}
