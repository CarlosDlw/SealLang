//! Emitting compile-time type properties.
//!
//! A `type_name` query interned a string is not needed: the concrete type name
//! becomes a private string global, exactly like a string literal. Layout
//! queries read the target's `LayoutCtxt`, so `size_of` and `align_of` agree
//! with every other layout decision the backend makes.

use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use seal_mir::TypeInfoKind;
use seal_ty::TyId;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Produces the value of a type-information query.
    pub(crate) fn type_info(
        &self,
        query: TypeInfoKind,
        ty: TyId,
        result_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match query {
            TypeInfoKind::Name => {
                let name = self.tcx.display_with(ty, self.interner).to_string();
                let (data, length) = self.constants.string_global(&name)?;
                let BasicTypeEnum::StructType(fat) = self.mapper.basic(result_ty)? else {
                    return Err(CodegenError::internal(
                        "a type name does not have a `*str` result type",
                    ));
                };
                Ok(fat.const_named_struct(&[data.into(), length.into()]).into())
            }
            TypeInfoKind::Size | TypeInfoKind::Align => {
                let layout = self.mapper.layout(ty)?;
                let value = if matches!(query, TypeInfoKind::Size) {
                    layout.size()
                } else {
                    layout.align()
                };
                Ok(self.mapper.usize_type()?.const_int(value, false).into())
            }
        }
    }
}
