//! LLVM module construction and code generation orchestration.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetTriple};
use inkwell::values::FunctionValue;
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{Body, Mir};
use seal_session::OptLevel;
use seal_span::{FileId, SourceMap};
use seal_symbol::Interner;
use seal_target::TargetSpec;
use seal_ty::TyId;

use crate::artifact;
use crate::constant::Constants;
use crate::debug::DebugContext;
use crate::entry;
use crate::error::CodegenError;
use crate::function::FunctionRequest;
use crate::names;
use crate::options::{CodegenOptions, Emit};
use crate::types::TypeMapper;

/// The checked program a code generation request consumes.
pub struct CodegenInput<'a> {
    /// Checked MIR for every function.
    pub mir: &'a Mir,
    /// HIR, used for aggregate definitions and debug names.
    pub hir: &'a Hir,
    /// Source map, used to resolve debug file names and positions.
    pub source_map: &'a SourceMap,
    /// Symbol interner.
    pub interner: &'a Interner,
    /// Target specification.
    pub target: &'a TargetSpec,
    /// The primary source file of this compilation unit.
    pub file: FileId,
}

/// The artifact produced by a successful code generation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    /// Where the artifact was written.
    pub path: PathBuf,
    /// What kind of artifact was produced.
    pub emit: Emit,
}

/// Lowers checked MIR to LLVM IR and produces the requested artifact.
pub fn emit(input: CodegenInput<'_>, options: &CodegenOptions) -> Result<Artifact, CodegenError> {
    let runtime = if options.emit == Emit::Executable {
        Some(runtime_archive(options, input.target)?)
    } else {
        None
    };
    Target::initialize_all(&InitializationConfig::default());
    let triple = TargetTriple::create(input.target.triple().as_str());
    let target = Target::from_triple(&triple).map_err(|error| {
        CodegenError::unsupported_target(
            input.target.triple().as_str(),
            format!("LLVM does not support this target: {error}"),
        )
    })?;
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            llvm_optimization(options.opt_level),
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| {
            CodegenError::unsupported_target(
                input.target.triple().as_str(),
                "LLVM could not create a target machine",
            )
        })?;

    let context = Context::create();
    let module = context.create_module(options.crate_name.as_deref().unwrap_or("seal"));
    module.set_triple(&triple);
    let data_layout = machine.get_target_data().get_data_layout();
    module.set_data_layout(&data_layout);
    let source_name = input
        .source_map
        .display_name(input.file)
        .unwrap_or_else(|| "seal-input".to_owned());
    module.set_source_file_name(&source_name);

    let mapper = TypeMapper::new(
        &context,
        input.hir.ty_ctxt(),
        input.interner,
        input.target.pointer_width(),
    )?;
    let debug = if options.debug_info {
        Some(DebugContext::new(
            &context,
            &module,
            input.source_map,
            input.hir.definitions(),
            input.hir.ty_ctxt(),
            &mapper,
            input.interner,
            input.file,
            options.opt_level != OptLevel::O0,
        )?)
    } else {
        None
    };

    let entry_point = if options.emit == Emit::Executable {
        Some(entry::validate(input.hir, input.mir)?)
    } else {
        None
    };

    let functions = declare_functions(&module, input.hir, input.mir, input.interner, &mapper)?;
    let builder = context.create_builder();
    let constants = Constants::new(&context, &module, &mapper, input.interner);

    for body in input.mir.bodies().iter() {
        let function = functions[&body.owner];
        let scope = match &debug {
            Some(debug) => {
                let linkage = names::function(
                    input
                        .interner
                        .resolve(input.hir.definitions().get(body.owner).name),
                );
                let scope = debug.subprogram(body, &linkage)?;
                function.set_subprogram(scope);
                Some(scope)
            }
            None => None,
        };
        FunctionRequest {
            context: &context,
            module: &module,
            builder: &builder,
            function,
            body,
            hir: input.hir,
            tcx: input.hir.ty_ctxt(),
            mapper: &mapper,
            constants: &constants,
            interner: input.interner,
            source_map: input.source_map,
            debug: debug.as_ref(),
            scope,
        }
        .lower()?;
    }

    if let Some(entry_point) = entry_point {
        entry::emit_wrapper(&context, &module, &builder, functions[&entry_point])?;
    }

    if let Some(debug) = &debug {
        debug.finalize();
    }
    module.verify().map_err(|error| {
        CodegenError::internal(format!("generated LLVM IR is invalid: {error}"))
    })?;

    if options.opt_level != OptLevel::O0 {
        module
            .run_passes(
                match options.opt_level {
                    OptLevel::O0 => "default<O0>",
                    OptLevel::O1 => "default<O1>",
                    OptLevel::O2 => "default<O2>",
                    OptLevel::O3 => "default<O3>",
                },
                &machine,
                PassBuilderOptions::create(),
            )
            .map_err(|error| {
                CodegenError::internal(format!("LLVM optimization failed: {error}"))
            })?;
    }

    artifact::emit_artifact(
        &module,
        &machine,
        options.emit,
        &options.output,
        runtime.as_deref().unwrap_or(&[]),
    )?;
    Ok(Artifact {
        path: options.output.clone(),
        emit: options.emit,
    })
}

/// Maximum size accepted for a user-supplied runtime archive.
const MAX_RUNTIME_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

fn runtime_archive(
    options: &CodegenOptions,
    target: &seal_target::TargetSpec,
) -> Result<Cow<'static, [u8]>, CodegenError> {
    if let Some(path) = &options.runtime_archive {
        let archive = read_runtime_archive(path, &options.output)?;
        return Ok(Cow::Owned(archive));
    }
    if !crate::runtime::supports(target) {
        return Err(CodegenError::artifact(
            &options.output,
            format!(
                "the embedded runtime was built for the host (`{}`), not for `{}`; \
                 pass --runtime-archive <PATH> with a runtime for the target, or use --emit=obj",
                seal_target::host_triple(),
                target
            ),
        ));
    }
    Ok(Cow::Borrowed(crate::runtime::ARCHIVE))
}

fn read_runtime_archive(path: &Path, output: &Path) -> Result<Vec<u8>, CodegenError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        CodegenError::artifact(
            output,
            format!("cannot read runtime archive `{}`: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(CodegenError::artifact(
            output,
            format!("runtime archive `{}` is not a regular file", path.display()),
        ));
    }
    if metadata.len() == 0 {
        return Err(CodegenError::artifact(
            output,
            format!("runtime archive `{}` is empty", path.display()),
        ));
    }
    if metadata.len() > MAX_RUNTIME_ARCHIVE_BYTES {
        return Err(CodegenError::artifact(
            output,
            format!(
                "runtime archive `{}` is too large ({} bytes; maximum is {MAX_RUNTIME_ARCHIVE_BYTES})",
                path.display(),
                metadata.len()
            ),
        ));
    }
    let archive = std::fs::read(path).map_err(|error| {
        CodegenError::artifact(
            output,
            format!("cannot read runtime archive `{}`: {error}", path.display()),
        )
    })?;
    if archive.is_empty() {
        return Err(CodegenError::artifact(
            output,
            format!("runtime archive `{}` is empty", path.display()),
        ));
    }
    if archive.len() as u64 > MAX_RUNTIME_ARCHIVE_BYTES {
        return Err(CodegenError::artifact(
            output,
            format!(
                "runtime archive `{}` is too large ({} bytes; maximum is {MAX_RUNTIME_ARCHIVE_BYTES})",
                path.display(),
                archive.len()
            ),
        ));
    }
    Ok(archive)
}

fn declare_functions<'ctx>(
    module: &Module<'ctx>,
    hir: &Hir,
    mir: &Mir,
    interner: &Interner,
    mapper: &TypeMapper<'ctx>,
) -> Result<FxHashMap<DefId, FunctionValue<'ctx>>, CodegenError> {
    let mut functions = FxHashMap::default();
    for body in mir.bodies().iter() {
        let definition = hir.definitions().get(body.owner);
        let name = names::function(interner.resolve(definition.name));
        let parameters: Vec<TyId> = body.args().map(|parameter| parameter.ty).collect();
        let returns = body.local(Body::return_local()).ty;
        let function_type = mapper.function_type(&parameters, returns)?;
        let function = module.add_function(&name, function_type, Some(Linkage::External));
        functions.insert(body.owner, function);
    }
    Ok(functions)
}

fn llvm_optimization(level: OptLevel) -> OptimizationLevel {
    match level {
        OptLevel::O0 => OptimizationLevel::None,
        OptLevel::O1 => OptimizationLevel::Less,
        OptLevel::O2 => OptimizationLevel::Default,
        OptLevel::O3 => OptimizationLevel::Aggressive,
    }
}
