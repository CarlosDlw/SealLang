//! Shared front-end harness for in-crate code generation tests.
//!
//! The tests inspect LLVM types and declared functions directly, so the harness
//! stops after monomorphization and exposes every intermediate value instead of
//! emitting an artifact.

use inkwell::context::Context;
use seal_borrowck::AliasingFacts;
use seal_hir::Hir;
use seal_mono::MonoProgram;
use seal_session::{Options, Session};
use seal_span::FileName;
use seal_ty::AdtId;

/// A fully checked and monomorphized program.
pub(crate) struct Compiled {
    /// Session owning the source map, interner, and target.
    pub(crate) session: Session,
    /// The lowered program.
    pub(crate) hir: Hir,
    /// Concrete instances reachable from the program's roots.
    pub(crate) mono: MonoProgram,
    /// Whole-program aliasing facts.
    pub(crate) facts: AliasingFacts,
}

/// Runs every phase up to and including monomorphization.
///
/// # Panics
///
/// Panics when any phase reports a diagnostic. A fixture that is expected to
/// fail belongs in a phase-specific test, not in this harness.
pub(crate) fn front_end(source: &str) -> Compiled {
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
    let typeck = seal_typeck::check(
        &hir,
        session.diag(),
        session.interner(),
        session.target().pointer_width(),
    );
    let mir = seal_mir::lower(&hir, &typeck);
    let facts = seal_borrowck::check(&hir, &mir, session.diag(), session.interner());
    let mono = seal_mono::collect(&hir, &mir, session.diag(), session.interner());
    assert!(
        !session.diag().has_errors(),
        "the fixture must check and monomorphize"
    );
    mono.validate(&hir)
        .expect("monomorphization preserves its invariants");
    Compiled {
        session,
        hir,
        mono,
        facts,
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
    crate::backend::declare_functions(
        &context,
        &module,
        &compiled.hir,
        &compiled.mono,
        compiled.session.interner(),
        &mapper,
        compiled.facts.noalias_functions(),
    )
    .expect("functions declare");
    module.print_to_string().to_string()
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
