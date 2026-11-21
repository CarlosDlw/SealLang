//! C ABI wrappers for `export "c" func` definitions.
//!
//! The Seal body keeps the internal language ABI. The wrapper has the C
//! signature computed by [`TypeMapper::c_signature`], converts coerced
//! aggregates at the boundary, and calls the already-lowered Seal function.
//! Keeping the body and the C entry separate means an export does not change
//! how Seal code calls the function or what a debugger sees inside it.

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, ValueKind};
use inkwell::{AddressSpace, DLLStorageClass};
use seal_hir::Hir;
use seal_mono::MonoProgram;
use seal_symbol::Interner;
use seal_ty::{Instance, TyId};

use crate::c_abi::{CParamKind, CReturn};
use crate::error::CodegenError;
use crate::extern_decl::apply_signature_attributes;
use crate::types::TypeMapper;

/// Emits one C wrapper for every exported function.
///
/// # Errors
///
/// Returns [`CodegenError`] when an export has no concrete instance, its
/// signature cannot be classified for the target, or LLVM rejects a conversion.
pub(crate) fn emit<'ctx>(
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    hir: &Hir,
    mono: &MonoProgram,
    mapper: &TypeMapper<'ctx>,
    functions: &seal_hash::FxHashMap<Instance, FunctionValue<'ctx>>,
    interner: &Interner,
) -> Result<(), CodegenError> {
    ExportEmitter {
        module,
        builder,
        hir,
        mono,
        mapper,
        functions,
        interner,
    }
    .run()
}

struct ExportEmitter<'ctx, 'a> {
    module: &'a Module<'ctx>,
    builder: &'a Builder<'ctx>,
    hir: &'a Hir,
    mono: &'a MonoProgram,
    mapper: &'a TypeMapper<'ctx>,
    functions: &'a seal_hash::FxHashMap<Instance, FunctionValue<'ctx>>,
    interner: &'a Interner,
}

impl<'ctx, 'a> ExportEmitter<'ctx, 'a> {
    fn run(&self) -> Result<(), CodegenError> {
        let context = self.mapper.context();
        let module = self.module;
        let builder = self.builder;
        let hir = self.hir;
        let mono = self.mono;
        let mapper = self.mapper;
        let interner = self.interner;
        let functions = self.functions;

        for &def in hir.exports() {
            let instance = mono
                .instances()
                .iter()
                .find(|candidate| {
                    candidate.instance.def == def
                        && hir
                            .ty_ctxt()
                            .substs_args(candidate.instance.args)
                            .is_empty()
                })
                .ok_or_else(|| {
                    CodegenError::internal(format!(
                        "the exported function {def:?} has no concrete instance"
                    ))
                })?;
            let inner = functions.get(&instance.instance).copied().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the exported function {def:?} has no lowered LLVM function"
                ))
            })?;
            let body = &instance.body;
            let parameters: Vec<TyId> = body.args().map(|parameter| parameter.ty).collect();
            let returns = body.local(seal_mir::Body::return_local()).ty;
            let signature = mapper.c_signature(&parameters, returns, false)?;
            let name = interner.resolve(hir.definitions().get(def).name);
            let wrapper =
                module.add_function(name, signature.function_type, Some(Linkage::External));
            apply_signature_attributes(module, wrapper, &signature);
            if module
                .get_triple()
                .as_str()
                .to_string_lossy()
                .contains("windows")
            {
                // A shared library only exports symbols marked with the PE
                // DLL storage class. Applying it to every export keeps the
                // static archive and a final DLL consistent.
                wrapper
                    .as_global_value()
                    .set_dll_storage_class(DLLStorageClass::Export);
            }

            let entry = context.append_basic_block(wrapper, "entry");
            builder.position_at_end(entry);
            let fixed = usize::from(matches!(&signature.ret, CReturn::Sret(_)));
            let mut arguments = Vec::with_capacity(parameters.len());
            for (index, &seal_ty) in parameters.iter().enumerate() {
                let value = wrapper
                    .get_nth_param((fixed + index) as u32)
                    .ok_or_else(|| {
                        CodegenError::internal(format!(
                            "the export `{name}` is missing C parameter {index}"
                        ))
                    })?;
                let converted = match signature.params.get(fixed + index) {
                    Some(parameter) => convert_parameter(
                        context,
                        builder,
                        mapper,
                        seal_ty,
                        value,
                        &parameter.kind,
                        name,
                    )?,
                    None => {
                        return Err(CodegenError::internal(format!(
                            "the export `{name}` has no C signature for parameter {index}"
                        )));
                    }
                };
                arguments.push(BasicMetadataValueEnum::from(converted));
            }

            let call = builder
                .build_call(inner, &arguments, "export.call")
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot build the export call `{name}`: {error}"
                    ))
                })?;
            match signature.ret {
                CReturn::Void => {
                    builder.build_return(None).map_err(|error| {
                        CodegenError::internal(format!("cannot return from `{name}`: {error}"))
                    })?;
                }
                CReturn::Direct => {
                    let ValueKind::Basic(value) = call.try_as_basic_value() else {
                        return Err(CodegenError::internal(format!(
                            "the export `{name}` returned no value"
                        )));
                    };
                    builder.build_return(Some(&value)).map_err(|error| {
                        CodegenError::internal(format!("cannot return from `{name}`: {error}"))
                    })?;
                }
                CReturn::Coerced(coerced) => {
                    let ValueKind::Basic(value) = call.try_as_basic_value() else {
                        return Err(CodegenError::internal(format!(
                            "the coerced export `{name}` returned no value"
                        )));
                    };
                    let aggregate = mapper.basic(returns)?;
                    let slot =
                        builder
                            .build_alloca(aggregate, "export.result")
                            .map_err(|error| {
                                CodegenError::internal(format!(
                                    "cannot allocate the export result `{name}`: {error}"
                                ))
                            })?;
                    builder.build_store(slot, value).map_err(|error| {
                        CodegenError::internal(format!(
                            "cannot store the export result `{name}`: {error}"
                        ))
                    })?;
                    let cast = builder
                        .build_pointer_cast(
                            slot,
                            context.ptr_type(AddressSpace::default()),
                            "export.cast",
                        )
                        .map_err(|error| {
                            CodegenError::internal(format!(
                                "cannot reinterpret the export result `{name}`: {error}"
                            ))
                        })?;
                    let result =
                        builder
                            .build_load(coerced, cast, "export.result")
                            .map_err(|error| {
                                CodegenError::internal(format!(
                                    "cannot load the export result `{name}`: {error}"
                                ))
                            })?;
                    builder.build_return(Some(&result)).map_err(|error| {
                        CodegenError::internal(format!("cannot return from `{name}`: {error}"))
                    })?;
                }
                CReturn::Sret(_) => {
                    let ValueKind::Basic(value) = call.try_as_basic_value() else {
                        return Err(CodegenError::internal(format!(
                            "the sret export `{name}` returned no value"
                        )));
                    };
                    let destination = wrapper
                        .get_nth_param(0)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "the sret export `{name}` has no destination"
                            ))
                        })?
                        .into_pointer_value();
                    builder.build_store(destination, value).map_err(|error| {
                        CodegenError::internal(format!(
                            "cannot store the sret export result `{name}`: {error}"
                        ))
                    })?;
                    builder.build_return(None).map_err(|error| {
                        CodegenError::internal(format!("cannot return from `{name}`: {error}"))
                    })?;
                }
            }
        }
        Ok(())
    }
}

fn convert_parameter<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    mapper: &TypeMapper<'ctx>,
    seal_ty: TyId,
    value: inkwell::values::BasicValueEnum<'ctx>,
    kind: &CParamKind<'ctx>,
    name: &str,
) -> Result<inkwell::values::BasicValueEnum<'ctx>, CodegenError> {
    match kind {
        CParamKind::Direct => Ok(value),
        CParamKind::ByPointer { aggregate, .. } => {
            let pointer = value.into_pointer_value();
            builder
                .build_load(*aggregate, pointer, "export.byval")
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot load the by-value parameter of `{name}`: {error}"
                    ))
                })
        }
        CParamKind::Coerced(coerced) => {
            let aggregate = mapper.basic(seal_ty)?;
            let slot = builder
                .build_alloca(*coerced, "export.parameter")
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot allocate the coerced parameter of `{name}`: {error}"
                    ))
                })?;
            builder.build_store(slot, value).map_err(|error| {
                CodegenError::internal(format!(
                    "cannot store the coerced parameter of `{name}`: {error}"
                ))
            })?;
            let cast = builder
                .build_pointer_cast(
                    slot,
                    context.ptr_type(AddressSpace::default()),
                    "export.parameter.cast",
                )
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot reinterpret the coerced parameter of `{name}`: {error}"
                    ))
                })?;
            builder
                .build_load(aggregate, cast, "export.parameter")
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot load the coerced parameter of `{name}`: {error}"
                    ))
                })
        }
    }
}
