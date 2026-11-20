//! Comparability rules for `==` and `!=`.
//!
//! Equality reads both operands and compares them by value, so a type is
//! comparable when every value it can hold is comparable: scalars, `char`,
//! `bool`, the unit type, aggregates of comparable values, and the contents of
//! `*str` and `*[T]`. Thin references are excluded because their only available
//! comparison is the address, which is almost never what the program means, and
//! trait objects because the concrete type is erased.

use seal_span::Span;
use seal_ty::{AdtKind, TyId, TyKind};

use super::Checker;

/// Why a type has no equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NotComparable {
    /// A thin reference would compare addresses instead of values.
    ThinReference,
    /// A trait object erases the concrete type.
    TraitObject,
    /// A type parameter has no known representation.
    Parameter,
    /// A function pointer compares addresses, which Seal does not expose.
    FunctionPointer,
    /// A field, array element, or slice element of this type is not comparable.
    Value(TyId),
}

impl Checker<'_> {
    /// Reports `op` when `ty` cannot be compared, and returns whether it can.
    pub(super) fn check_comparable(&self, op: &str, ty: TyId, span: Span) -> bool {
        match self.comparability(ty) {
            Ok(()) => true,
            Err(reason) => {
                self.report_not_comparable(op, ty, reason, span);
                false
            }
        }
    }

    /// Returns whether `ty` supports `==` and `!=`.
    fn comparability(&self, ty: TyId) -> Result<(), NotComparable> {
        let tcx = self.hir.ty_ctxt();
        match tcx.kind(ty) {
            // `Error` is already diagnosed, and an unresolved variable cannot be
            // judged yet; treating both as comparable keeps one mistake from
            // cascading into unrelated equality errors.
            TyKind::Bool
            | TyKind::Char
            | TyKind::Int(_)
            | TyKind::Uint(_)
            | TyKind::Float(_)
            | TyKind::Unit
            | TyKind::Never
            | TyKind::Error
            | TyKind::Infer(_) => Ok(()),
            TyKind::Ref { pointee, .. } => match tcx.kind(pointee) {
                TyKind::Str => Ok(()),
                TyKind::Slice { element } => self.comparability(element),
                TyKind::Dyn { .. } => Err(NotComparable::TraitObject),
                _ => Err(NotComparable::ThinReference),
            },
            // Raw pointers compare addresses, which is exactly their equality
            // contract; the metadata does not participate.
            TyKind::RawPtr { .. } => Ok(()),
            TyKind::Array { element, .. } => self.comparability(element),
            TyKind::Tuple(tuple) => {
                for element in tcx.tuple_elements(tuple) {
                    self.comparability_field(element)?;
                }
                Ok(())
            }
            TyKind::Adt { adt, args } => {
                let definition = tcx.adt(adt);
                match definition.kind {
                    AdtKind::Struct => {
                        for field in &definition.fields {
                            self.comparability_field(tcx.substitute(field.ty, args))?;
                        }
                    }
                    AdtKind::Enum => {
                        for variant in &definition.variants {
                            for field in &variant.fields {
                                self.comparability_field(tcx.substitute(field.ty, args))?;
                            }
                        }
                    }
                    // A closure has no equality operation of its own; type
                    // checking rejects the comparison before code generation.
                    AdtKind::Closure => return Err(NotComparable::Value(ty)),
                }
                Ok(())
            }
            TyKind::FnPtr(_) => Err(NotComparable::FunctionPointer),
            TyKind::Param(_) => Err(NotComparable::Parameter),
            // An unsized value cannot be an operand, so these only appear while
            // recovering from another error.
            TyKind::Str
            | TyKind::Slice { .. }
            | TyKind::Dyn { .. }
            | TyKind::Opaque { .. }
            | TyKind::FnSig(_) => Err(NotComparable::Value(ty)),
        }
    }

    /// Requires a nested value to be comparable, naming it in the reason.
    fn comparability_field(&self, field: TyId) -> Result<(), NotComparable> {
        self.comparability(field)
            .map_err(|_| NotComparable::Value(field))
    }

    fn report_not_comparable(&self, op: &str, ty: TyId, reason: NotComparable, span: Span) {
        let help = match reason {
            NotComparable::ThinReference => {
                "a reference compares its address, not its pointee; compare the value or its fields"
            }
            NotComparable::TraitObject => {
                "a trait object erases the concrete type, so its values have no equality; compare a concrete value"
            }
            NotComparable::Parameter => {
                "a type parameter has no equality; compare a value of a concrete type"
            }
            NotComparable::FunctionPointer => {
                "a function pointer has no `==`; compare the result of calling it, or store an explicit tag"
            }
            NotComparable::Value(culprit) => {
                let name = self.ty_text(culprit);
                self.error(
                    seal_error_codes::E3002,
                    span,
                    format!(
                        "operator `{op}` cannot be applied to type `{}`",
                        self.ty_text(ty)
                    ),
                )
                .help(format!(
                    "the value of type `{name}` inside it has no equality"
                ))
                .emit();
                return;
            }
        };
        self.error(
            seal_error_codes::E3002,
            span,
            format!(
                "operator `{op}` cannot be applied to type `{}`",
                self.ty_text(ty)
            ),
        )
        .help(help)
        .emit();
    }
}
