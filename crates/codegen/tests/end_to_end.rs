//! End-to-end code generation tests.
//!
//! Each test runs the full front end, checks the MIR invariants, lowers to
//! LLVM, and exercises the produced artifact. Executable tests assert the
//! language-level exit status; artifact tests inspect the emitted file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use seal_codegen::{CodegenError, CodegenInput, CodegenOptions, Emit, EntryPointError};
use seal_session::{OptLevel, Options, Session};
use seal_span::FileName;

const STRUCTS: &str = include_str!("../../../examples/006_structs.sl");

static NEXT_OUTPUT: AtomicU32 = AtomicU32::new(0);

struct TempOutput(PathBuf);

impl TempOutput {
    fn new(suffix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-codegen-test-{}-{}-{suffix}",
            std::process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn compile(
    source: &str,
    output: &Path,
    emit: Emit,
    opt_level: OptLevel,
    debug_info: bool,
) -> Result<(), CodegenError> {
    let session = Session::new(Options::default()).expect("session starts");
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

    let mir = seal_mir::lower(&hir, &typeck);
    mir.validate()
        .expect("MIR lowering preserves its invariants");
    seal_borrowck::check(&hir, &mir, session.diag(), session.interner());
    assert!(!session.diag().has_errors(), "borrow checking failed");

    seal_codegen::emit(
        CodegenInput {
            mir: &mir,
            hir: &hir,
            source_map: session.source_map(),
            interner: session.interner(),
            target: session.target(),
            file,
        },
        &CodegenOptions {
            emit,
            output: output.to_path_buf(),
            opt_level,
            debug_info,
            crate_name: Some("test".to_owned()),
        },
    )
    .map(|_| ())
}

fn run(path: &Path) -> std::process::ExitStatus {
    Command::new(path).status().expect("artifact runs")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn debug_and_release_executables_produce_the_language_result() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "debug"),
        (OptLevel::O2, false, "release"),
        (OptLevel::O2, true, "release-debug"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            STRUCTS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("code generation succeeds");
        assert_eq!(
            run(output.path()).code(),
            Some(10),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn debug_executable_contains_dwarf_types_functions_and_variables() {
    let output = TempOutput::new("dwarf");
    compile(STRUCTS, output.path(), Emit::Executable, OptLevel::O0, true)
        .expect("code generation succeeds");

    let bytes = std::fs::read(output.path()).expect("artifact is readable");
    for section in [
        b".debug_info".as_slice(),
        b".debug_abbrev".as_slice(),
        b".debug_line".as_slice(),
        b".debug_str".as_slice(),
    ] {
        assert!(
            contains(&bytes, section),
            "missing {}",
            String::from_utf8_lossy(section)
        );
    }

    // The dump is a supplemental check; the section assertions above are the
    // deterministic contract. CI images that ship LLVM have the tool installed.
    if let Ok(dump) = Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(output.path())
        .output()
    {
        assert!(dump.status.success(), "llvm-dwarfdump rejects the artifact");
        let text = String::from_utf8_lossy(&dump.stdout);
        for expected in [
            "DW_TAG_subprogram",
            "DW_TAG_variable",
            "\"main\"",
            "\"segment\"",
        ] {
            assert!(text.contains(expected), "debug info is missing {expected}");
        }
    }
}

#[test]
fn object_and_assembly_output_do_not_require_an_entry_point() {
    let source = "func add(a: i32, b: i32) -> i32 { a + b }";

    let object = TempOutput::new("helper.o");
    compile(source, object.path(), Emit::Object, OptLevel::O2, false)
        .expect("object emission succeeds");
    assert!(
        std::fs::metadata(object.path())
            .expect("object exists")
            .len()
            > 0
    );

    let assembly = TempOutput::new("helper.s");
    compile(source, assembly.path(), Emit::Assembly, OptLevel::O2, false)
        .expect("assembly emission succeeds");
    let text = std::fs::read_to_string(assembly.path()).expect("assembly is UTF-8");
    assert!(
        text.contains(".globl"),
        "assembly has no global function directive"
    );
    assert!(text.contains("seal.add"), "assembly lost the symbol name");
}

#[test]
fn entry_point_errors_are_reported() {
    let output = TempOutput::new("entry");
    let missing = compile(
        "func helper() -> i32 { 1 }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect_err("a program without main cannot be linked");
    assert!(matches!(
        missing,
        CodegenError::EntryPoint(EntryPointError::Missing)
    ));

    let parameters = compile(
        "func main(value: i32) -> i32 { value }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect_err("main cannot take parameters");
    assert!(matches!(
        parameters,
        CodegenError::EntryPoint(EntryPointError::Parameters(1))
    ));

    let returns = compile(
        "func main() -> i64 { 0 }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect_err("main must return i32 or unit");
    assert!(matches!(
        returns,
        CodegenError::EntryPoint(EntryPointError::ReturnType(_))
    ));
}

#[test]
fn a_unit_main_exits_with_zero() {
    let output = TempOutput::new("unit-main");
    compile(
        "func main() { let value = 1; }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        true,
    )
    .expect("unit main compiles");
    assert_eq!(run(output.path()).code(), Some(0));
}

#[test]
fn float_comparisons_select_the_expected_branch() {
    let output = TempOutput::new("float");
    compile(
        "func main() -> i32 { let a: f32 = 1.5; let b: f32 = 2.0; if a < b { 0 } else { 1 } }",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("float comparison compiles");
    assert_eq!(run(output.path()).code(), Some(0));
}

#[test]
fn string_literals_lower_to_fat_pointers() {
    let output = TempOutput::new("string");
    compile(
        "func ignore(text: *str) -> i32 { 0 }\nfunc main() -> i32 { ignore(\"seal\") }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("string literal compiles");
    assert_eq!(run(output.path()).code(), Some(0));
}

#[cfg(unix)]
#[test]
fn integer_division_by_zero_traps() {
    use std::os::unix::process::ExitStatusExt;

    let output = TempOutput::new("division");
    compile(
        "func main() -> i32 { let zero = 0; 10 / zero }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("division compiles");
    let status = run(output.path());
    assert!(
        status.signal().is_some(),
        "division by zero must trap, got {status:?}"
    );
}
