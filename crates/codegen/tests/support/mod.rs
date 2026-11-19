//! Shared harness for code generation integration tests.
//!
//! Every test runs the real front end (lex, parse, HIR, type checking, MIR,
//! borrow checking, and monomorphization) before lowering, so a code generation
//! result that depends on an earlier invariant cannot be tested in isolation.
//!
//! Not every test binary uses every helper, hence the module-wide allowance.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use seal_codegen::{CodegenError, CodegenInput, CodegenOptions, Emit};
use seal_session::{OptLevel, Options, Session};
use seal_span::FileName;

static NEXT_OUTPUT: AtomicU32 = AtomicU32::new(0);

/// A unique temporary artifact path removed on drop.
pub(crate) struct TempOutput(PathBuf);

impl TempOutput {
    /// Reserves a unique path with `suffix` for this process.
    pub(crate) fn new(suffix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-codegen-test-{}-{}-{suffix}",
            std::process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }

    /// Returns the reserved path.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Compiles `source` for the host target.
pub(crate) fn compile(
    source: &str,
    output: &Path,
    emit: Emit,
    opt_level: OptLevel,
    debug_info: bool,
) -> Result<(), CodegenError> {
    compile_for_target(
        source,
        output,
        emit,
        opt_level,
        debug_info,
        seal_target::host_triple(),
    )
}

/// Compiles `source` for `target`.
pub(crate) fn compile_for_target(
    source: &str,
    output: &Path,
    emit: Emit,
    opt_level: OptLevel,
    debug_info: bool,
    target: seal_target::TargetTriple,
) -> Result<(), CodegenError> {
    compile_for_target_with_runtime(source, output, emit, opt_level, debug_info, target, None)
}

/// Compiles `source` for `target`, using `runtime_archive` when linking.
pub(crate) fn compile_for_target_with_runtime(
    source: &str,
    output: &Path,
    emit: Emit,
    opt_level: OptLevel,
    debug_info: bool,
    target: seal_target::TargetTriple,
    runtime_archive: Option<PathBuf>,
) -> Result<(), CodegenError> {
    let session = Session::new(Options {
        target,
        ..Options::default()
    })
    .expect("session starts");
    let file = session
        .add_file(FileName::virtual_name("test.sl"), source.to_owned())
        .unwrap_or_else(|_| panic!("source registers"));
    let source_file = session
        .source_map()
        .file(file)
        .expect("registered source exists");

    let tokens = seal_lexer::lex(&source_file, session.diag(), session.interner());
    let ast = seal_parser::parse(&tokens, session.diag(), session.interner());
    let hir = seal_hir::lower(&ast, session.diag(), session.interner());
    assert!(
        !session.diag().has_errors(),
        "lexing, parsing, or HIR failed"
    );

    let typeck = seal_typeck::check(
        &hir,
        session.diag(),
        session.interner(),
        session.target().pointer_width(),
    );
    assert!(!session.diag().has_errors(), "type checking failed");

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
    let aliasing = seal_borrowck::check(
        &hir,
        &mir,
        Some(&constants),
        session.diag(),
        session.interner(),
    );
    assert!(!session.diag().has_errors(), "borrow checking failed");

    // Monomorphization diagnostics are not asserted here: a generic `main` is
    // reported by collection and then surfaces through entry-point validation,
    // which is exactly what the entry-point test exercises. Every successful
    // test still asserts the code generation result below.
    let mono = seal_mono::collect(&hir, &mir, &constants, session.diag(), session.interner());
    if !session.diag().has_errors() {
        mono.validate(&hir)
            .expect("monomorphization preserves its invariants");
        mono.validate_globals(&constants)
            .expect("monomorphization preserves its global invariants");
    }

    seal_codegen::emit(
        CodegenInput {
            mono: &mono,
            constants: &constants,
            hir: &hir,
            source_map: session.source_map(),
            interner: session.interner(),
            target: session.target(),
            file,
            noalias_functions: aliasing.noalias_functions(),
        },
        &CodegenOptions {
            emit,
            output: output.to_path_buf(),
            opt_level,
            debug_info,
            crate_name: Some("test".to_owned()),
            runtime_archive,
        },
    )
    .map(|_| ())
}

/// Runs an executable and returns its exit status.
pub(crate) fn run(path: &Path) -> std::process::ExitStatus {
    Command::new(path).status().expect("artifact runs")
}

/// Runs an executable and captures its output.
pub(crate) fn run_captured(path: &Path) -> std::process::Output {
    Command::new(path).output().expect("artifact runs")
}

/// Returns `true` when `haystack` contains `needle`.
pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Returns the section directive in effect at `symbol`'s label.
pub(crate) fn section_of(assembly: &str, symbol: &str) -> Option<String> {
    let mut section: Option<String> = None;
    for line in assembly.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(".section") {
            let name = rest.split(',').next().unwrap_or_default().trim();
            section = Some(name.to_owned());
        } else if matches!(line, ".data" | ".text" | ".bss") {
            section = Some(line.to_owned());
        } else if line.trim_end_matches(':') == symbol {
            return section;
        }
    }
    None
}

/// Returns `true` for a section that is never written after loading.
pub(crate) fn is_read_only(section: &str) -> bool {
    section.contains("rodata")
        || section.contains("data.rel.ro")
        || section.contains("__const")
        || section.contains("__DATA_CONST")
}

/// Asserts that `assembly` has no thread-local storage and no constructors.
pub(crate) fn assert_no_thread_locals_or_constructors(assembly: &str) {
    assert!(
        !assembly.contains(".tdata") && !assembly.contains(".tbss"),
        "a global must not become thread-local:\n{assembly}"
    );
    assert!(
        !assembly.contains("llvm.global_ctors"),
        "globals must be initialized without a constructor:\n{assembly}"
    );
}
