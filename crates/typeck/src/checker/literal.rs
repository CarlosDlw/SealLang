//! Literal typing and range checking.

use seal_ast::{Literal, PrimitiveType};
use seal_hir::primitive_kind;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{TyId, TyKind};

use super::Checker;

impl Checker<'_> {
    /// Types a literal, adopting `expected` when the literal can have it.
    ///
    /// `allow_signed_min` lets the operand of unary `-` name the most negative
    /// value of a signed type, which is not representable as a positive literal.
    pub(super) fn literal_ty(
        &self,
        literal: Literal,
        expected: Option<TyId>,
        span: Span,
        allow_signed_min: bool,
    ) -> TyId {
        if let Some(expected) = expected
            && self.is_error(expected)
        {
            return expected;
        }

        match literal {
            Literal::Int { value, suffix, .. } => {
                let ty = self.integer_literal_ty(suffix, expected);
                if self.is_error(ty) {
                    return ty;
                }
                if !self.integer_literal_fits(value, ty, allow_signed_min) {
                    self.report_integer_range(value, ty, span);
                    return self.error_ty();
                }
                ty
            }
            Literal::Float { value, suffix } => {
                let ty = self.float_literal_ty(suffix, expected);
                if self.is_error(ty) {
                    return ty;
                }
                if matches!(
                    self.hir.ty_ctxt().kind(ty),
                    TyKind::Float(seal_ty::FloatTy::F32)
                ) && !(value as f32).is_finite()
                {
                    self.error(
                        seal_error_codes::E3009,
                        span,
                        "floating-point literal is out of range for `f32`",
                    )
                    .help("use an `f64` literal or a smaller value")
                    .emit();
                    return self.error_ty();
                }
                ty
            }
            Literal::Str(_) => self.str_ref_ty(),
            Literal::Char(_) => self.interned(TyKind::Char),
            Literal::Bool(_) => self.bool_ty(),
        }
    }

    fn integer_literal_ty(&self, suffix: Option<Symbol>, expected: Option<TyId>) -> TyId {
        if let Some(ty) = suffix.and_then(|suffix| self.suffix_ty(suffix, true)) {
            return ty;
        }
        if let Some(expected) = expected
            && self.is_integer(expected)
        {
            return expected;
        }
        self.i32_ty()
    }

    fn float_literal_ty(&self, suffix: Option<Symbol>, expected: Option<TyId>) -> TyId {
        if let Some(ty) = suffix.and_then(|suffix| self.suffix_ty(suffix, false)) {
            return ty;
        }
        if let Some(expected) = expected
            && self.is_float(expected)
        {
            return expected;
        }
        self.f64_ty()
    }

    fn suffix_ty(&self, suffix: Symbol, integer: bool) -> Option<TyId> {
        let name = self.interner.resolve(suffix);
        let primitive = PrimitiveType::from_name(name)?;
        let kind = primitive_kind(primitive);
        let matches_family = if integer {
            matches!(kind, TyKind::Int(_) | TyKind::Uint(_))
        } else {
            matches!(kind, TyKind::Float(_))
        };
        if matches_family {
            self.hir.ty_ctxt().lookup(kind)
        } else {
            None
        }
    }

    fn integer_literal_fits(&self, value: u128, ty: TyId, allow_signed_min: bool) -> bool {
        let bits = self.integer_bits(ty).unwrap_or(self.pointer_bits());
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Int(_) => {
                let unsigned_max = unsigned_max(bits);
                let max = if bits >= 128 {
                    i128::MAX as u128
                } else {
                    unsigned_max >> 1
                };
                let max = if allow_signed_min {
                    max.saturating_add(1)
                } else {
                    max
                };
                value <= max
            }
            TyKind::Uint(_) => value <= unsigned_max(bits),
            _ => false,
        }
    }

    fn report_integer_range(&self, value: u128, ty: TyId, span: Span) {
        self.error(
            seal_error_codes::E3009,
            span,
            format!(
                "integer literal `{value}` is out of range for `{}`",
                self.ty_text(ty)
            ),
        )
        .help("choose a wider type or a smaller value")
        .emit();
    }
}

fn unsigned_max(bits: u16) -> u128 {
    if bits >= 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}
