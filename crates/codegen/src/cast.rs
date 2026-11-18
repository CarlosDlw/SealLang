//! Code generation for explicit numeric casts.
//!
//! Each conversion is a single LLVM instruction, except float-to-integer,
//! which calls the saturating intrinsic. Plain `fptosi`/`fptoui` is undefined
//! for out-of-range values, NaN, and infinity, so it is never emitted for a
//! user cast.

use inkwell::intrinsics::Intrinsic;
use inkwell::types::{BasicTypeEnum, IntType};
use inkwell::values::{BasicValueEnum, ValueKind};
use seal_mir::Operand;
use seal_ty::{NumericCastKind, TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Lowers one numeric cast to its LLVM conversion.
    pub(crate) fn cast(
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
