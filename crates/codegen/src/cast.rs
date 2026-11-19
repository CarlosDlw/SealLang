//! Code generation for explicit casts.
//!
//! Each numeric conversion is a single LLVM instruction, except
//! float-to-integer, which calls the saturating intrinsic. Plain
//! `fptosi`/`fptoui` is undefined for out-of-range values, NaN, and infinity,
//! so it is never emitted for a user cast. Pointer conversions preserve the
//! value representation, except that converting a fat pointer to an integer
//! reads its data word.

use inkwell::intrinsics::Intrinsic;
use inkwell::types::{BasicTypeEnum, IntType};
use inkwell::values::{BasicValueEnum, ValueKind};
use seal_mir::Operand;
use seal_ty::{CastKind, NumericCastKind, TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Lowers one explicit cast to its LLVM conversion.
    pub(crate) fn cast(
        &self,
        kind: CastKind,
        operand: &Operand,
        from: TyId,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match kind {
            CastKind::Numeric(kind) => self.numeric_cast(kind, operand, from, to),
            // A reference and a raw pointer share their LLVM representation, and
            // a thin reference that unsizes to a trait object is lowered to
            // `MakeTraitObject` before code generation.
            CastKind::RefToRaw { .. } | CastKind::RawToRaw => self.load_operand(operand),
            CastKind::RawToInt => self.cast_raw_to_int(operand, to),
            CastKind::IntToRaw => self.cast_int_to_raw(operand, to),
            CastKind::RawToRef => Err(CodegenError::internal(
                "a raw-pointer-to-reference cast reached code generation",
            )),
        }
    }

    /// Lowers one numeric cast to its LLVM conversion.
    fn numeric_cast(
        &self,
        kind: NumericCastKind,
        operand: &Operand,
        from: TyId,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let value = self.load_operand(operand)?;
        match kind {
            NumericCastKind::IntToInt => self.cast_int_to_int(value, from, to),
            NumericCastKind::IntToFloat => self.cast_int_to_float(value, from, to),
            NumericCastKind::FloatToInt => self.cast_float_to_int(value, to),
            NumericCastKind::FloatToFloat => self.cast_float_to_float(value, to),
        }
    }

    /// Converts a raw pointer to a pointer-sized integer.
    ///
    /// A fat pointer carries a data word and a metadata word; the cast exposes
    /// the data address and drops the metadata.
    fn cast_raw_to_int(
        &self,
        operand: &Operand,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let value = self.load_operand(operand)?;
        let pointer = match value {
            BasicValueEnum::PointerValue(pointer) => pointer,
            BasicValueEnum::StructValue(fat) => self
                .build(
                    self.builder.build_extract_value(fat, 0, "cast.data"),
                    "raw pointer data",
                )?
                .into_pointer_value(),
            other => {
                return Err(CodegenError::internal(format!(
                    "a raw-pointer-to-integer cast received {other:?}"
                )));
            }
        };
        let target = self.int_target(to, "raw-pointer-to-integer cast")?;
        let converted = self
            .builder
            .build_ptr_to_int(pointer, target, "cast.ptrtoint");
        Ok(self.build(converted, "raw pointer to integer cast")?.into())
    }

    /// Converts a pointer-sized integer to a thin raw pointer.
    fn cast_int_to_raw(
        &self,
        operand: &Operand,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let BasicValueEnum::IntValue(value) = self.load_operand(operand)? else {
            return Err(CodegenError::internal(
                "an integer-to-raw-pointer cast received a non-integer operand",
            ));
        };
        let _ = to;
        let converted =
            self.builder
                .build_int_to_ptr(value, self.mapper.pointer(), "cast.inttoptr");
        Ok(self.build(converted, "integer to raw pointer cast")?.into())
    }

    fn cast_int_to_int(
        &self,
        value: BasicValueEnum<'ctx>,
        from: TyId,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let BasicValueEnum::IntValue(value) = value else {
            return Err(CodegenError::internal(
                "an integer-to-integer cast received a non-integer operand",
            ));
        };
        let target = self.int_target(to, "integer-to-integer cast")?;
        let source_width = value.get_type().get_bit_width();
        let target_width = target.get_bit_width();
        if source_width == target_width {
            return Ok(value.into());
        }
        let signed = matches!(self.tcx.kind(from), TyKind::Int(_));
        let converted = if target_width < source_width {
            self.builder.build_int_truncate(value, target, "cast.trunc")
        } else {
            self.builder
                .build_int_cast_sign_flag(value, target, signed, "cast.extend")
        };
        Ok(self.build(converted, "integer cast")?.into())
    }

    fn cast_int_to_float(
        &self,
        value: BasicValueEnum<'ctx>,
        from: TyId,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let BasicValueEnum::IntValue(value) = value else {
            return Err(CodegenError::internal(
                "an integer-to-float cast received a non-integer operand",
            ));
        };
        let BasicTypeEnum::FloatType(target) = self.mapper.basic(to)? else {
            return Err(CodegenError::internal(
                "an integer-to-float cast has a non-float target type",
            ));
        };
        let signed = matches!(self.tcx.kind(from), TyKind::Int(_));
        let converted = if signed {
            self.builder
                .build_signed_int_to_float(value, target, "cast.sitofp")
        } else {
            self.builder
                .build_unsigned_int_to_float(value, target, "cast.uitofp")
        };
        Ok(self.build(converted, "integer to float cast")?.into())
    }

    fn cast_float_to_float(
        &self,
        value: BasicValueEnum<'ctx>,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let BasicValueEnum::FloatValue(value) = value else {
            return Err(CodegenError::internal(
                "a float-to-float cast received a non-float operand",
            ));
        };
        let BasicTypeEnum::FloatType(target) = self.mapper.basic(to)? else {
            return Err(CodegenError::internal(
                "a float-to-float cast has a non-float target type",
            ));
        };
        let converted = self.builder.build_float_cast(value, target, "cast.float");
        Ok(self.build(converted, "float cast")?.into())
    }

    fn cast_float_to_int(
        &self,
        value: BasicValueEnum<'ctx>,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let BasicValueEnum::FloatValue(value) = value else {
            return Err(CodegenError::internal(
                "a float-to-integer cast received a non-float operand",
            ));
        };
        let target = self.int_target(to, "float-to-integer cast")?;
        let signed = matches!(self.tcx.kind(to), TyKind::Int(_));
        let name = if signed {
            "llvm.fptosi.sat"
        } else {
            "llvm.fptoui.sat"
        };
        let intrinsic = Intrinsic::find(name).ok_or_else(|| {
            CodegenError::internal(format!("LLVM does not provide the `{name}` intrinsic"))
        })?;
        let declaration = intrinsic
            .get_declaration(self.module, &[target.into(), value.get_type().into()])
            .ok_or_else(|| {
                CodegenError::internal(format!("cannot declare the `{name}` intrinsic"))
            })?;
        let call = self.build(
            self.builder
                .build_call(declaration, &[value.into()], "cast.sat"),
            "saturating float to integer cast",
        )?;
        match call.try_as_basic_value() {
            ValueKind::Basic(BasicValueEnum::IntValue(value)) => Ok(value.into()),
            _ => Err(CodegenError::internal(
                "the saturating float-to-integer intrinsic returned no integer",
            )),
        }
    }

    fn int_target(&self, ty: TyId, operation: &str) -> Result<IntType<'ctx>, CodegenError> {
        let BasicTypeEnum::IntType(target) = self.mapper.basic(ty)? else {
            return Err(CodegenError::internal(format!(
                "a {operation} has a non-integer target type"
            )));
        };
        Ok(target)
    }
}
