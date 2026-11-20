//! LLVM module construction and code generation orchestration.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use inkwell::OptimizationLevel;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetTriple};
use inkwell::values::FunctionValue;
use seal_const_eval::ConstProgram;
use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;
use seal_mir::Body;
use seal_mono::MonoProgram;
use seal_session::OptLevel;
use seal_span::{FileId, SourceMap};
use seal_symbol::Interner;
use seal_target::TargetSpec;
use seal_ty::{Instance, RefKind, TyId, TyKind};

use crate::artifact;
use crate::const_value::ConstValues;
use crate::constant::Constants;
use crate::debug::DebugContext;
use crate::entry;
use crate::error::CodegenError;
use crate::function::FunctionRequest;
use crate::global::Globals;
use crate::names;
use crate::options::{CodegenOptions, Emit};
use crate::relocation::Pointers;
use crate::types::TypeMapper;
use crate::vtable::Vtables;

/// The checked program a code generation request consumes.
pub struct CodegenInput<'a> {
    /// The monomorphized program: one concrete body per reachable instance.
    ///
    /// Code generation never reads generic MIR. Every type in these bodies has
    /// its instance arguments substituted and every call names a collected
    /// instance, so no parameter or symbolic trait call can reach LLVM.
    pub mono: &'a MonoProgram,
    /// The evaluated constants: static storage, const allocations, and the
    /// value of every `const` item a body reads.
    ///
    /// Code generation never evaluates source. A constant that the evaluator did
    /// not produce is an internal invariant violation, not a user error.
    pub constants: &'a ConstProgram,
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
    /// Functions whose exclusive-reference parameters may be marked `noalias`.
    ///
    /// The set comes from borrow checking's [`AliasingFacts`], which proves that
    /// no call site lets another reference name the pointee. Marking a parameter
    /// outside this set would let LLVM assume aliasing that safe code allows.
    ///
    /// [`AliasingFacts`]: seal_borrowck::AliasingFacts
    pub noalias_functions: &'a FxHashSet<DefId>,
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

    let mapper =
        TypeMapper::new_for_target(&context, input.hir.ty_ctxt(), input.interner, input.target)?;
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
        Some(entry::validate(input.hir, input.mono)?)
    } else {
        None
    };

    // Dynamic calls add call sites the static aliasing proof never saw, so any
    // method reachable through a vtable is excluded before declarations are
    // annotated. The input set stays untouched.
    let noalias = crate::vtable::filtered_noalias(input.mono, input.noalias_functions);
    let externals =
        crate::extern_decl::Externals::declare(&module, input.hir, &mapper, input.interner)?;
    if let Some(debug) = &debug {
        for &def in input.hir.extern_functions() {
            let Some(declaration) = input.hir.extern_function(def) else {
                continue;
            };
            let function = externals.function(def).ok_or_else(|| {
                CodegenError::internal(format!("external function {def:?} was not declared"))
            })?;
            let parameters: Vec<TyId> = declaration.params.iter().map(|param| param.ty).collect();
            debug.declare_extern_function(def, function, &parameters, declaration.ret)?;
        }
    }
    let functions = declare_functions(
        &context,
        &module,
        input.hir,
        input.mono,
        input.interner,
        &mapper,
        &noalias,
    )?;
    // Drop glue is declared and filled before vtables, whose first slot points
    // at the concrete type's glue, and before bodies, whose drop statements call
    // it.
    let drop_glues =
        crate::drop_glue::DropGlues::declare(&context, &module, input.hir, input.mono)?;
    drop_glues.emit(&context, input.hir, &mapper, input.mono, &functions)?;
    // Vtable globals point at glue and method functions, so both must exist
    // before the globals are initialized.
    let vtables = Vtables::declare(
        &module,
        input.hir,
        input.mono,
        &mapper,
        &functions,
        &drop_glues,
    )?;
    let builder = context.create_builder();
    let constants = Constants::new(&context, &module, &mapper, input.interner);
    // Globals are declared before any initializer is built, so a relocation may
    // name a global that appears later in the program. Vtable globals already
    // exist, because a constant trait object points at one.
    let values = ConstValues::new(
        &context,
        input.hir,
        &mapper,
        input.constants,
        input.interner,
        input.target.endianness(),
    );
    let globals = Globals::declare(
        &module,
        input.hir,
        input.mono,
        input.constants,
        &values,
        input.interner,
    )?;
    let pointers = Pointers::new(&context, &constants, &vtables, &globals);
    globals.initialize(&values, &pointers)?;
    if let Some(debug) = &debug {
        for (def, global, ty) in globals.statics() {
            debug.declare_static(def, global, ty)?;
        }
    }

    for mono in input.mono.instances().iter() {
        let body = &mono.body;
        let function = functions[&mono.instance];
        let scope = match &debug {
            Some(debug) => {
                let name = seal_mono::instance_name(input.hir, input.interner, mono.instance);
                let linkage = symbol(input.hir, input.interner, mono.instance);
                let scope = debug.subprogram(body, &name, &linkage)?;
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
            values: &values,
            pointers: &pointers,
            globals: &globals,
            externals: &externals,
            functions: &functions,
            vtables: &vtables,
            drop_glues: &drop_glues,
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

    let source_libraries: Vec<String> = input
        .hir
        .link_directives()
        .iter()
        .map(|directive| input.interner.resolve(directive.name).to_owned())
        .collect();
    artifact::emit_artifact(
        &module,
        &machine,
        options,
        runtime.as_deref().unwrap_or(&[]),
        &source_libraries,
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
                 pass `sealc build --runtime-archive <PATH>` with a runtime for the target, \
                 or use `sealc build --emit=obj`",
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

pub(crate) fn declare_functions<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    hir: &Hir,
    mono: &MonoProgram,
    interner: &Interner,
    mapper: &TypeMapper<'ctx>,
    noalias_functions: &FxHashSet<DefId>,
) -> Result<FxHashMap<Instance, FunctionValue<'ctx>>, CodegenError> {
    let mut functions = FxHashMap::default();
    for mono in mono.instances().iter() {
        let body = &mono.body;
        let name = symbol(hir, interner, mono.instance);
        let parameters: Vec<TyId> = body.args().map(|parameter| parameter.ty).collect();
        let returns = body.local(Body::return_local()).ty;
        let function_type = mapper.function_type(&parameters, returns)?;
        let function = module.add_function(&name, function_type, Some(Linkage::External));
        add_reference_attributes(
            context,
            function,
            hir,
            body,
            noalias_functions.contains(&body.owner),
        );
        functions.insert(mono.instance, function);
    }
    Ok(functions)
}

/// Returns the linker symbol of one instance.
///
/// An instance without type arguments keeps the symbol the definition always
/// had. A generic instance appends the stable instance fingerprint, which
/// covers the definition and every type argument, so two instantiations cannot
/// collide and repeated builds produce the same symbol.
pub(crate) fn symbol(hir: &Hir, interner: &Interner, instance: Instance) -> String {
    let definition = hir.definitions().get(instance.def);
    let parent = definition
        .parent
        .map(|parent| interner.resolve(hir.definitions().get(parent).name));
    let base = names::function(interner.resolve(definition.name), parent);
    if hir.ty_ctxt().substs_args(instance.args).is_empty() {
        base
    } else {
        names::instance(&base, seal_mono::instance_hash(hir, instance))
    }
}

/// Adds the attributes every reference parameter can carry.
///
/// A shared `*T` parameter is always `readonly`: the type system forbids writing
/// through it, so the promise holds without a whole-program proof. An exclusive
/// `*mut T` parameter is `noalias` only when the facts proved that the callee
/// never reaches the pointee through an unknown pointer and that every call site
/// keeps the argument disjoint from every other reference. A `*str`, `*mut str`,
/// `*dyn Trait`, or `*mut dyn Trait` parameter is a fat pointer, which LLVM
/// cannot annotate, so it keeps the ordinary ABI.
fn add_reference_attributes<'ctx>(
    context: &'ctx Context,
    function: FunctionValue<'ctx>,
    hir: &Hir,
    body: &Body,
    noalias: bool,
) {
    for (index, parameter) in body.args().enumerate() {
        let TyKind::Ref { kind, pointee } = hir.ty_ctxt().kind(parameter.ty) else {
            continue;
        };
        if matches!(
            hir.ty_ctxt().kind(pointee),
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. }
        ) {
            continue;
        }
        let attribute = match kind {
            RefKind::Shared => Some("readonly"),
            RefKind::Exclusive if noalias => Some("noalias"),
            RefKind::Exclusive => None,
        };
        if let Some(attribute) = attribute {
            let kind = Attribute::get_named_enum_kind_id(attribute);
            let attribute = context.create_enum_attribute(kind, 0);
            function.add_attribute(AttributeLoc::Param(index as u32), attribute);
        }
    }
}

fn llvm_optimization(level: OptLevel) -> OptimizationLevel {
    match level {
        OptLevel::O0 => OptimizationLevel::None,
        OptLevel::O1 => OptimizationLevel::Less,
        OptLevel::O2 => OptimizationLevel::Default,
        OptLevel::O3 => OptimizationLevel::Aggressive,
    }
}
