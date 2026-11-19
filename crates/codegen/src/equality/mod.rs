//! Structural and content equality.
//!
//! `==` and `!=` compare values rather than addresses: aggregates field by
//! field, `*str` by its bytes, and `*[T]` by its elements. Both operands are
//! materialized on the stack and the comparison walks the language layout, so
//! one routine covers scalars nested inside aggregates and never reads the
//! padding that the layout leaves between fields.
//!
//! Enum payloads are compared only for the active variant: the bytes of the
//! other variants are uninitialized, and reading them would be undefined.

mod address;
mod field;
mod sequence;

use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Emits `lhs == rhs`, or `lhs != rhs` when `negate` is set.
    ///
    /// The operands are values of `ty`; calling this for a type without
    /// equality is an internal error, because type checking rejects it first.
    pub(crate) fn equality(
        &self,
        ty: TyId,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        negate: bool,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let value_type = self.mapper.basic(ty)?;
        let lhs_slot = self.entry_alloca(value_type, "equality.lhs")?;
        let rhs_slot = self.entry_alloca(value_type, "equality.rhs")?;
        self.build(
            self.builder.build_store(lhs_slot, lhs),
            "equality lhs store",
        )?;
        self.build(
            self.builder.build_store(rhs_slot, rhs),
            "equality rhs store",
        )?;
        let equal = self.equal_memory(ty, lhs_slot, rhs_slot, span)?;
        if negate {
            Ok(self
                .build(self.builder.build_not(equal, "inequal"), "inequality")?
                .into())
        } else {
            Ok(equal.into())
        }
    }

    /// Compares the values of type `ty` stored at `lhs` and `rhs`.
    fn equal_memory(
        &self,
        ty: TyId,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        match self.tcx.kind(ty) {
            TyKind::Unit | TyKind::Never | TyKind::Error => Ok(self.true_value()),
            TyKind::Bool | TyKind::Char | TyKind::Int(_) | TyKind::Uint(_) | TyKind::Float(_) => {
                let value_type = self.mapper.basic(ty)?;
                let lhs = self.build(
                    self.builder.build_load(value_type, lhs, "equality lhs"),
                    "equality load",
                )?;
                let rhs = self.build(
                    self.builder.build_load(value_type, rhs, "equality rhs"),
                    "equality load",
                )?;
                Ok(self
                    .binary(seal_mir::BinOp::Eq, lhs, rhs, ty, span)?
                    .into_int_value())
            }
            TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                TyKind::Str => self.equal_str(ty, lhs, rhs, span),
                TyKind::Slice { element } => self.equal_slice(ty, element, lhs, rhs, span),
                TyKind::Dyn { .. } => Err(CodegenError::internal(
                    "a trait object has no equality in code generation",
                )),
                _ => Err(CodegenError::internal(
                    "a thin reference has no equality in code generation",
                )),
            },
            TyKind::Array { element, length } => {
                let count = self.mapper.usize_type()?.const_int(length, false);
                self.equal_elements(element, lhs, rhs, count, span)
            }
            TyKind::Tuple(tuple) => {
                let layouts = self.mapper.tuple_field_layouts(ty)?;
                let elements = self.tcx.tuple_elements(tuple);
                if layouts.len() != elements.len() {
                    return Err(CodegenError::internal(
                        "a tuple's layout does not match its element count",
                    ));
                }
                let fields: Vec<(TyId, u64)> = elements
                    .iter()
                    .zip(layouts)
                    .map(|(element, layout)| (*element, layout.offset))
                    .collect();
                self.equal_fields(&fields, lhs, rhs, span)
            }
            TyKind::Adt { adt, args } => {
                let definition = self.tcx.adt(adt);
                match definition.kind {
                    seal_ty::AdtKind::Struct => {
                        let layouts = self.mapper.field_layouts(adt, args)?;
                        let fields: Vec<(TyId, u64)> = definition
                            .fields
                            .iter()
                            .zip(layouts)
                            .map(|(field, layout)| {
                                (self.tcx.substitute(field.ty, args), layout.offset)
                            })
                            .collect();
                        self.equal_fields(&fields, lhs, rhs, span)
                    }
                    seal_ty::AdtKind::Enum => self.equal_enum(adt, args, lhs, rhs, span),
                }
            }
            TyKind::RawPtr { pointee, .. } => {
                let value_type = self.mapper.basic(ty)?;
                let lhs = self.build(
                    self.builder.build_load(value_type, lhs, "raw lhs"),
                    "raw lhs load",
                )?;
                let rhs = self.build(
                    self.builder.build_load(value_type, rhs, "raw rhs"),
                    "raw rhs load",
                )?;
                let fat = matches!(
                    self.tcx.kind(pointee),
                    TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. }
                );
                let lhs = if fat {
                    self.build(
                        self.builder.build_extract_value(
                            lhs.into_struct_value(),
                            0,
                            "raw lhs data",
                        ),
                        "raw lhs data",
                    )?
                    .into_pointer_value()
                } else {
                    lhs.into_pointer_value()
                };
                let rhs = if fat {
                    self.build(
                        self.builder.build_extract_value(
                            rhs.into_struct_value(),
                            0,
                            "raw rhs data",
                        ),
                        "raw rhs data",
                    )?
                    .into_pointer_value()
                } else {
                    rhs.into_pointer_value()
                };
                let usize_type = self.mapper.usize_type()?;
                let lhs = self.build(
                    self.builder
                        .build_ptr_to_int(lhs, usize_type, "raw lhs address"),
                    "raw lhs address",
                )?;
                let rhs = self.build(
                    self.builder
                        .build_ptr_to_int(rhs, usize_type, "raw rhs address"),
                    "raw rhs address",
                )?;
                Ok(self.build(
                    self.builder
                        .build_int_compare(inkwell::IntPredicate::EQ, lhs, rhs, "raw eq"),
                    "raw pointer equality",
                )?)
            }
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. } | TyKind::Param(_) => Err(
                CodegenError::internal("an unsized or uninstantiated type has no equality"),
            ),
            TyKind::Infer(_) => Err(CodegenError::internal(
                "an unresolved inference type reached an equality comparison",
            )),
        }
    }

    /// Returns the constant `true`, the neutral element of a field comparison.
    fn true_value(&self) -> IntValue<'ctx> {
        self.context.bool_type().const_int(1, false)
    }
}
