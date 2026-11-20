//! LLVM declarations for functions and statics implemented by C.
//!
//! Every declaration is emitted once, under its resolved linker symbol, and
//! cached by definition. Code generation never re-reads the source to find a
//! symbol, and a call never depends on the order in which bodies are lowered.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::module::{Linkage, Module};
use inkwell::types::{AnyTypeEnum, BasicTypeEnum};
use inkwell::values::{FunctionValue, GlobalValue};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_symbol::Interner;
use seal_ty::TyId;

use crate::c_abi::{CParamKind, CReturn, CSignature};
use crate::error::CodegenError;
use crate::types::TypeMapper;

/// Declared external functions and statics, keyed by their HIR definition.
pub(crate) struct Externals<'ctx> {
    functions: FxHashMap<DefId, (FunctionValue<'ctx>, CSignature<'ctx>)>,
    statics: FxHashMap<DefId, (GlobalValue<'ctx>, TyId)>,
}

impl<'ctx> Externals<'ctx> {
    /// Declares every external function and static of the program.
    ///
    /// Declarations carry `Linkage::External` and no attributes: the compiler
    /// does not own their bodies and cannot promise `noalias`, `readonly`, or
    /// `nounwind`.
    pub(crate) fn declare(
        module: &Module<'ctx>,
        hir: &Hir,
        mapper: &TypeMapper<'ctx>,
        interner: &Interner,
    ) -> Result<Self, CodegenError> {
        let mut functions = FxHashMap::default();
        for &def in hir.extern_functions() {
            let Some(data) = hir.extern_function(def) else {
                continue;
            };
            let name = interner.resolve(data.symbol);
            let parameters: Vec<TyId> = data.params.iter().map(|param| param.ty).collect();
            let returns = data.ret.unwrap_or_else(|| hir.ty_ctxt().unit_ty());
            let signature = mapper.c_signature(&parameters, returns, data.variadic)?;
            let function =
                module.add_function(name, signature.function_type, Some(Linkage::External));
            apply_signature_attributes(module, function, &signature);
            functions.insert(def, (function, signature));
        }

        let mut statics = FxHashMap::default();
        for &def in hir.extern_statics() {
            let Some(data) = hir.extern_static(def) else {
                continue;
            };
            let name = interner.resolve(data.symbol);
            let global = module.add_global(mapper.basic(data.ty)?, None, name);
            global.set_linkage(Linkage::External);
            // C owns the storage: it is never a `constant` global, even when
            // Seal declares it immutable, because C may initialize it at load
            // time.
            global.set_constant(false);
            statics.insert(def, (global, data.ty));
        }

        Ok(Self { functions, statics })
    }

    /// Returns the declaration of `def`, if it is an external function.
    pub(crate) fn function(&self, def: DefId) -> Option<FunctionValue<'ctx>> {
        self.functions.get(&def).map(|(function, _)| *function)
    }

    /// Returns the C signature of `def`, if it is an external function.
    pub(crate) fn signature(&self, def: DefId) -> Option<&CSignature<'ctx>> {
        self.functions.get(&def).map(|(_, signature)| signature)
    }

    /// Returns the global and its type for an external static.
    pub(crate) fn static_global(&self, def: DefId) -> Option<(GlobalValue<'ctx>, TyId)> {
        self.statics.get(&def).copied()
    }
}

/// Applies `byval` and `sret` to the parameters that need them.
///
/// Both attributes carry the aggregate type in modern LLVM, so they are type
/// attributes rather than enum attributes. Without `byval`, LLVM would pass a
/// large System V struct pointer in a register instead of copying it to the
/// stack as C expects.
pub(crate) fn apply_signature_attributes(
    module: &Module<'_>,
    function: FunctionValue<'_>,
    signature: &CSignature<'_>,
) {
    let context = module.get_context();
    for (index, parameter) in signature.params.iter().enumerate() {
        let CParamKind::ByPointer {
            byval: true,
            aggregate,
        } = parameter.kind
        else {
            continue;
        };
        let kind = Attribute::get_named_enum_kind_id("byval");
        let attribute = context.create_type_attribute(kind, any_type(aggregate));
        function.add_attribute(AttributeLoc::Param(index as u32), attribute);
    }
    if let CReturn::Sret(aggregate) = &signature.ret {
        let kind = Attribute::get_named_enum_kind_id("sret");
        let attribute = context.create_type_attribute(kind, any_type(*aggregate));
        function.add_attribute(AttributeLoc::Param(0), attribute);
    }
}

fn any_type(ty: BasicTypeEnum<'_>) -> AnyTypeEnum<'_> {
    match ty {
        BasicTypeEnum::ArrayType(value) => value.into(),
        BasicTypeEnum::FloatType(value) => value.into(),
        BasicTypeEnum::IntType(value) => value.into(),
        BasicTypeEnum::PointerType(value) => value.into(),
        BasicTypeEnum::StructType(value) => value.into(),
        BasicTypeEnum::VectorType(value) => value.into(),
        BasicTypeEnum::ScalableVectorType(value) => value.into(),
    }
}
