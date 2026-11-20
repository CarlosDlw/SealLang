//! Shared front-end harness for in-crate code generation tests.
//!
//! The tests inspect LLVM types and declared functions directly, so the harness
//! stops after monomorphization and exposes every intermediate value instead of
//! emitting an artifact.

use inkwell::context::Context;
use inkwell::module::Module;
use seal_borrowck::AliasingFacts;
use seal_def::DefKind;
use seal_hir::Hir;
use seal_mono::MonoProgram;
use seal_session::{Options, Session};
use seal_span::{FileId, FileName};
use seal_ty::AdtId;

/// A fully checked and monomorphized program.
pub(crate) struct Compiled {
    /// Session owning the source map, interner, and target.
    pub(crate) session: Session,
    /// The lowered program.
    pub(crate) hir: Hir,
    /// Concrete instances reachable from the program's roots.
    pub(crate) mono: MonoProgram,
    /// Every evaluated constant, static, and allocation.
    pub(crate) constants: seal_const_eval::ConstProgram,
    /// Whole-program aliasing facts.
    pub(crate) facts: AliasingFacts,
    /// The primary source file.
    pub(crate) file: FileId,
}

/// Runs every phase up to and including monomorphization.
///
/// # Panics
///
/// Panics when any phase reports a diagnostic. A fixture that is expected to
/// fail belongs in a phase-specific test, not in this harness.
pub(crate) fn front_end(source: &str) -> Compiled {
    front_end_with_destructors(source, &[])
}

/// Runs every phase up to and including monomorphization after registering a
/// destructor for each named aggregate.
///
/// `impl Drop` is not part of the surface yet, so a test that needs a concrete
/// type to need dropping registers one before MIR lowering, which is exactly
/// where HIR lowering will record it once the syntax exists. Each pair is the
/// aggregate and the function standing in for its destructor.
pub(crate) fn front_end_with_destructors(source: &str, destructors: &[(&str, &str)]) -> Compiled {
    let session = Session::new(Options::default()).expect("session starts");
    let file = session
        .add_file(FileName::virtual_name("test.sl"), source.to_owned())
        .expect("source registers");
    let source_file = session
        .source_map()
        .file(file)
        .expect("registered source exists");

    let tokens = seal_lexer::lex(&source_file, session.diag(), session.interner());
    let ast = seal_parser::parse(&tokens, session.diag(), session.interner());
    let hir = seal_hir::lower(&ast, session.diag(), session.interner());
    for (aggregate, destructor) in destructors {
        let aggregate_symbol = session
            .interner()
            .lookup(aggregate)
            .unwrap_or_else(|| panic!("`{aggregate}` is never interned"));
        let adt = hir
            .definitions()
            .iter_enumerated()
            .find(|(_, data)| {
                matches!(data.kind, DefKind::Struct | DefKind::Enum)
                    && data.name == aggregate_symbol
            })
            .and_then(|(id, _)| hir.adt(id))
            .unwrap_or_else(|| panic!("no aggregate named `{aggregate}`"));
        let destructor_symbol = session
            .interner()
            .lookup(destructor)
            .unwrap_or_else(|| panic!("`{destructor}` is never interned"));
        let method = hir
            .definitions()
            .iter_enumerated()
            .find(|(_, data)| data.kind == DefKind::Function && data.name == destructor_symbol)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no function named `{destructor}`"));
        assert_eq!(
            hir.ty_ctxt().set_drop_method(adt, method),
            None,
            "`{aggregate}` starts without a destructor"
        );
    }
    let typeck = seal_typeck::check(
        &hir,
        session.diag(),
        session.interner(),
        session.target().pointer_width(),
    );
    let mut mir = seal_mir::lower(&hir, &typeck);
    mir.validate()
        .expect("MIR lowering preserves its invariants");
    seal_mir::elaborate_drops(&hir, &mut mir).expect("drop elaboration preserves its invariants");
    mir.validate_with(&hir)
        .expect("drop elaboration preserves the MIR invariants");
    let constants = seal_const_eval::evaluate(
        &hir,
        &mir,
        session.diag(),
        session.interner(),
        session.target().pointer_width(),
    );
    let facts = seal_borrowck::check(
        &hir,
        &mir,
        Some(&constants),
        session.diag(),
        session.interner(),
    );
    let mono = seal_mono::collect(&hir, &mir, &constants, session.diag(), session.interner());
    assert!(
        !session.diag().has_errors(),
        "the fixture must check and monomorphize"
    );
    mono.validate(&hir)
        .expect("monomorphization preserves its invariants");
    mono.validate_globals(&constants)
        .expect("monomorphization preserves its global invariants");
    Compiled {
        session,
        hir,
        mono,
        constants,
        facts,
        file,
    }
}

/// Returns the IR text of every declared instance's function.
pub(crate) fn module_ir(compiled: &Compiled) -> String {
    let context = Context::create();
    let module = context.create_module("test");
    let mapper = crate::types::TypeMapper::new(
        &context,
        compiled.hir.ty_ctxt(),
        compiled.session.interner(),
        compiled.session.target().pointer_width(),
    )
    .expect("types map to LLVM");
    let noalias =
        crate::vtable::filtered_noalias(&compiled.mono, compiled.facts.noalias_functions());
    crate::backend::declare_functions(
        &context,
        &module,
        &compiled.hir,
        &compiled.mono,
        compiled.session.interner(),
        &mapper,
        &noalias,
    )
    .expect("functions declare");
    module.print_to_string().to_string()
}

/// Declares functions and vtables and lowers every body into `context`.
///
/// The returned module owns the complete IR a production module would contain
/// before target-machine emission, which lets tests inspect vtable globals and
/// call instructions without writing an artifact. Debug information is built
/// when `debug_info` is set, mirroring the production pipeline.
pub(crate) fn build_module<'ctx>(
    compiled: &'ctx Compiled,
    context: &'ctx Context,
    debug_info: bool,
) -> Module<'ctx> {
    let module = context.create_module("test");
    let mapper = crate::types::TypeMapper::new(
        context,
        compiled.hir.ty_ctxt(),
        compiled.session.interner(),
        compiled.session.target().pointer_width(),
    )
    .expect("types map to LLVM");
    let noalias =
        crate::vtable::filtered_noalias(&compiled.mono, compiled.facts.noalias_functions());
    let functions = crate::backend::declare_functions(
        context,
        &module,
        &compiled.hir,
        &compiled.mono,
        compiled.session.interner(),
        &mapper,
        &noalias,
    )
    .expect("functions declare");
    let glues =
        crate::drop_glue::DropGlues::declare(context, &module, &compiled.hir, &compiled.mono)
            .expect("drop glue declares");
    glues
        .emit(context, &compiled.hir, &mapper, &compiled.mono, &functions)
        .expect("drop glue emits");
    let vtables = crate::vtable::Vtables::declare(
        &module,
        &compiled.hir,
        &compiled.mono,
        &mapper,
        &functions,
        &glues,
    )
    .expect("vtables emit");
    let debug = if debug_info {
        Some(
            crate::debug::DebugContext::new(
                context,
                &module,
                compiled.session.source_map(),
                compiled.hir.definitions(),
                compiled.hir.ty_ctxt(),
                &mapper,
                compiled.session.interner(),
                compiled.file,
                false,
            )
            .expect("debug information starts"),
        )
    } else {
        None
    };
    let builder = context.create_builder();
    let constants =
        crate::constant::Constants::new(context, &module, &mapper, compiled.session.interner());
    let values = crate::const_value::ConstValues::new(
        context,
        &compiled.hir,
        &mapper,
        &compiled.constants,
        compiled.session.interner(),
        compiled.session.target().endianness(),
    );
    let globals = crate::global::Globals::declare(
        &module,
        &compiled.hir,
        &compiled.mono,
        &compiled.constants,
        &values,
        compiled.session.interner(),
    )
    .expect("globals declare");
    let pointers = crate::relocation::Pointers::new(context, &constants, &vtables, &globals);
    globals
        .initialize(&values, &pointers)
        .expect("globals initialize");
    if let Some(debug) = &debug {
        for (def, global, ty) in globals.statics() {
            debug
                .declare_static(def, global, ty)
                .expect("a static is described");
        }
    }
    let externals = crate::extern_decl::Externals::declare(
        &module,
        &compiled.hir,
        &mapper,
        compiled.session.interner(),
    )
    .expect("externals declare");
    for mono in compiled.mono.instances().iter() {
        let scope = debug.as_ref().map(|debug| {
            let name =
                seal_mono::instance_name(&compiled.hir, compiled.session.interner(), mono.instance);
            let linkage =
                crate::backend::symbol(&compiled.hir, compiled.session.interner(), mono.instance);
            let scope = debug
                .subprogram(&mono.body, &name, &linkage)
                .expect("the subprogram describes its instance");
            functions[&mono.instance].set_subprogram(scope);
            scope
        });
        crate::function::FunctionRequest {
            context,
            module: &module,
            builder: &builder,
            function: functions[&mono.instance],
            body: &mono.body,
            hir: &compiled.hir,
            tcx: compiled.hir.ty_ctxt(),
            mapper: &mapper,
            constants: &constants,
            values: &values,
            pointers: &pointers,
            globals: &globals,
            externals: &externals,
            functions: &functions,
            vtables: &vtables,
            drop_glues: &glues,
            interner: compiled.session.interner(),
            source_map: compiled.session.source_map(),
            debug: debug.as_ref(),
            scope,
        }
        .lower()
        .expect("a body lowers");
    }
    if let Some(debug) = &debug {
        debug.finalize();
    }
    module
}

/// Returns the complete IR of a compiled program, including vtables and bodies.
pub(crate) fn lowered_module_ir(compiled: &Compiled) -> String {
    let context = Context::create();
    let module = build_module(compiled, &context, false);
    module.print_to_string().to_string()
}

/// Returns complete IR with debug metadata for a compiled program.
pub(crate) fn lowered_debug_module_ir(compiled: &Compiled) -> String {
    let context = Context::create();
    let module = build_module(compiled, &context, true);
    module.print_to_string().to_string()
}

/// Returns one vtable global's IR name for the only coercion in a fixture.
pub(crate) fn vtable_name(compiled: &Compiled) -> String {
    let table = compiled
        .mono
        .vtables()
        .iter()
        .next()
        .expect("the fixture collects one vtable");
    crate::names::vtable(seal_mono::vtable_hash(&compiled.hir, &table.key))
}

/// Returns the IR of one function, including its full body.
pub(crate) fn function_ir(ir: &str, name: &str) -> String {
    let marker = format!("@{name}(");
    let mut lines = ir.lines();
    while let Some(line) = lines.next() {
        let declaration = line.starts_with("define ") || line.starts_with("declare ");
        if !declaration || !line.contains(&marker) {
            continue;
        }
        if line.starts_with("declare ") {
            return line.to_owned();
        }
        let mut function = line.to_owned();
        for line in lines.by_ref() {
            function.push('\n');
            function.push_str(line);
            if line == "}" {
                return function;
            }
        }
    }
    panic!("function `{name}` is not in the module IR")
}

/// Resolves the vtable for a coercion, returning the internal error when the
/// monomorphized program has no table for it.
pub(crate) fn resolve_vtable(
    compiled: &Compiled,
    source_ty: seal_ty::TyId,
    target_ty: seal_ty::TyId,
) -> Result<(), crate::CodegenError> {
    let context = Context::create();
    let module = context.create_module("test");
    let mapper = crate::types::TypeMapper::new(
        &context,
        compiled.hir.ty_ctxt(),
        compiled.session.interner(),
        compiled.session.target().pointer_width(),
    )
    .expect("types map to LLVM");
    let noalias =
        crate::vtable::filtered_noalias(&compiled.mono, compiled.facts.noalias_functions());
    let functions = crate::backend::declare_functions(
        &context,
        &module,
        &compiled.hir,
        &compiled.mono,
        compiled.session.interner(),
        &mapper,
        &noalias,
    )
    .expect("functions declare");
    let glues =
        crate::drop_glue::DropGlues::declare(&context, &module, &compiled.hir, &compiled.mono)
            .expect("drop glue declares");
    let vtables = crate::vtable::Vtables::declare(
        &module,
        &compiled.hir,
        &compiled.mono,
        &mapper,
        &functions,
        &glues,
    )
    .expect("vtables emit");
    vtables
        .global_for_coercion(compiled.hir.ty_ctxt(), source_ty, target_ty)
        .map(|_| ())
}

/// Returns the aggregate named `name`, which must have exactly one definition.
pub(crate) fn adt(compiled: &Compiled, name: &str) -> AdtId {
    let symbol = compiled
        .session
        .interner()
        .lookup(name)
        .unwrap_or_else(|| panic!("`{name}` is never interned"));
    let found: Vec<AdtId> = compiled
        .hir
        .definitions()
        .iter_enumerated()
        .filter(|(_, definition)| definition.name == symbol)
        .filter_map(|(id, _)| compiled.hir.adt(id))
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one aggregate `{name}`");
    found[0]
}
