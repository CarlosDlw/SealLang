//! Reading operands.

use inkwell::values::BasicValueEnum;
use seal_mir::{Constant, Operand};
use seal_ty::TyId;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Returns the type of the value read by `operand`.
    pub(crate) fn operand_type(&self, operand: &Operand) -> Result<TyId, CodegenError> {
        Ok(match operand {
            Operand::Copy(place) | Operand::Move(place) => self.place_type(place)?,
            Operand::Const(def) => self
                .hir
                .body_of(*def)
                .and_then(|body| body.ret)
                .ok_or_else(|| {
                    CodegenError::internal(format!("constant operand {def:?} has no declared type"))
                })?,
            Operand::Constant(constant) => match constant {
                Constant::Int { ty, .. }
                | Constant::Float { ty, .. }
                | Constant::Str { ty, .. }
                | Constant::CStr { ty, .. } => *ty,
                Constant::Bool(_) => self.tcx.bool_ty(),
                Constant::Char(_) => self.tcx.char_ty(),
                Constant::Unit => self.tcx.unit_ty(),
            },
        })
    }

    /// Loads the value read by `operand`.
    pub(crate) fn load_operand(
        &self,
        operand: &Operand,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match operand {
            Operand::Constant(constant) => self.constants.emit(constant),
            // A value constant is read from one private global holding its
            // evaluated memory image, so aggregate constants with relocations
            // reach the body as ordinary loads of the language type.
            Operand::Const(def) => {
                let global = self.values.value_global(self.module, *def, self.pointers)?;
                let ty = self.mapper.basic(self.operand_type(operand)?)?;
                self.build(
                    self.builder
                        .build_load(ty, global.as_pointer_value(), "constant value"),
                    "constant load",
                )
            }
            Operand::Copy(place) | Operand::Move(place) => {
                let pointer = self.place_ptr(place)?;
                let ty = self.mapper.basic(self.place_type(place)?)?;
                self.build(self.builder.build_load(ty, pointer, "load"), "operand load")
            }
        }
    }
}
