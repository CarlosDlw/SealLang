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
const RUNTIME_PANIC: &str = include_str!("../../../examples/007_runtime_panic.sl");
const EXCLUSIVE: &str = include_str!("../../../examples/008_exclusive_references.sl");

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
    compile_for_target(
        source,
        output,
        emit,
        opt_level,
        debug_info,
        seal_target::host_triple(),
    )
}

fn compile_for_target(
    source: &str,
    output: &Path,
    emit: Emit,
    opt_level: OptLevel,
    debug_info: bool,
    target: seal_target::TargetTriple,
) -> Result<(), CodegenError> {
    compile_for_target_with_runtime(source, output, emit, opt_level, debug_info, target, None)
}

fn compile_for_target_with_runtime(
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
            runtime_archive,
        },
    )
    .map(|_| ())
}

fn run(path: &Path) -> std::process::ExitStatus {
    Command::new(path).status().expect("artifact runs")
}

fn run_captured(path: &Path) -> std::process::Output {
    Command::new(path).output().expect("artifact runs")
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
fn a_dereference_of_a_returned_reference_runs() {
    let source = "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
                  func main() -> i32 {\n  let x = 41;\n  *identity(&x) + 1\n}\n";
    let output = TempOutput::new("deref-reference-value");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("code generation succeeds");
    assert_eq!(run(output.path()).code(), Some(42));
}

#[test]
fn a_borrow_through_a_reference_value_runs() {
    let source = "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
                  func main() -> i32 {\n  let x = 41;\n  let r = &*identity(&x);\n  *r + 1\n}\n";
    let output = TempOutput::new("borrow-reference-value");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("code generation succeeds");
    assert_eq!(run(output.path()).code(), Some(42));
}

#[test]
fn exclusive_references_write_through_the_pointer() {
    let output = TempOutput::new("exclusive-references");
    compile(
        EXCLUSIVE,
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("code generation succeeds");
    assert_eq!(run(output.path()).code(), Some(43));
}

#[test]
fn exclusive_field_writes_run() {
    let source = "struct Point {\n  x: i32,\n  y: i32,\n}\n\n\
                  func fill(point: *mut Point) {\n  (*point).x = 40;\n  (*point).y = 2;\n}\n\n\
                  func main() -> i32 {\n  let mut point = Point { x: 0, y: 0 };\n  fill(&mut point);\n  point.x + point.y\n}\n";
    let output = TempOutput::new("exclusive-fields");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("code generation succeeds");
    assert_eq!(run(output.path()).code(), Some(42));
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

#[test]
fn cross_target_executables_require_a_matching_runtime() {
    let triple = if cfg!(target_arch = "x86_64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let target = seal_target::TargetTriple::parse(triple).expect("test triple is valid");
    let source = "func main() -> i32 { 0 }";

    let executable = TempOutput::new("cross");
    let error = compile_for_target(
        source,
        executable.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        target.clone(),
    )
    .expect_err("an executable needs a runtime for its target");
    match error {
        CodegenError::Artifact { detail, .. } => {
            assert!(detail.contains("embedded runtime"));
            assert!(detail.contains("--runtime-archive"));
            assert!(detail.contains("--emit=obj"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let object = TempOutput::new("cross.o");
    compile_for_target(
        source,
        object.path(),
        Emit::Object,
        OptLevel::O0,
        false,
        target,
    )
    .expect("object emission does not need the runtime");
    assert!(
        std::fs::metadata(object.path())
            .expect("object exists")
            .len()
            > 0
    );
}

#[test]
fn external_runtime_archives_are_read_and_validated() {
    let source = "func main() -> i32 { 0 }";
    let host = seal_target::host_triple();

    let missing = TempOutput::new("missing-runtime.a");
    let _ = std::fs::remove_file(missing.path());
    let error = compile_for_target_with_runtime(
        source,
        missing.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        host.clone(),
        Some(missing.path().to_path_buf()),
    )
    .expect_err("a missing runtime archive cannot be read");
    match error {
        CodegenError::Artifact { detail, .. } => {
            assert!(detail.contains("cannot read runtime archive"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let empty = TempOutput::new("empty-runtime.a");
    std::fs::write(empty.path(), []).expect("empty archive is written");
    let error = compile_for_target_with_runtime(
        source,
        empty.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        host,
        Some(empty.path().to_path_buf()),
    )
    .expect_err("an empty runtime archive is rejected");
    match error {
        CodegenError::Artifact { detail, .. } => {
            assert!(detail.contains("is empty"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn division_by_zero_panics_with_the_source_location() {
    let output = TempOutput::new("division-zero");
    compile(
        "// one\n// two\nfunc main() -> i32 {\n    let zero = 0;\n    10 / zero\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("division compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to divide by zero at <test.sl>:5:5\n"
    );
}

#[test]
fn nested_division_panics_at_the_expression_span() {
    let output = TempOutput::new("nested-division-panic");
    compile(
        "func main() -> i32 {\n    let zero = 0;\n    1 + 10 / zero\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("the nested division compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to divide by zero at <test.sl>:3:9\n"
    );
}

#[test]
fn runtime_panic_example_matches_the_documented_contract() {
    let output = TempOutput::new("runtime-panic-example");
    compile(
        RUNTIME_PANIC,
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("the runtime panic example compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to divide by zero at <test.sl>:5:3\n"
    );
}

#[test]
fn signed_division_overflow_panics() {
    let output = TempOutput::new("division-overflow");
    compile(
        "func main() -> i32 {\n    let minimum: i32 = -2147483647 - 1;\n    let minus_one: i32 = -1;\n    minimum / minus_one\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("division compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to divide with overflow at <test.sl>:4:5\n"
    );
}

#[test]
fn remainder_by_zero_panics() {
    let output = TempOutput::new("remainder-zero");
    compile(
        "// one\n// two\n// three\n// four\n// five\n// six\n// seven\n// eight\n// nine\nfunc main() -> i32 {\n    let zero = 0;\n    10 % zero\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect("remainder compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to calculate the remainder with a divisor of zero at <test.sl>:12:5\n"
    );
}

#[test]
fn signed_remainder_overflow_panics() {
    let output = TempOutput::new("remainder-overflow");
    compile(
        "func main() -> i32 {\n    let minimum: i32 = -2147483647 - 1;\n    let minus_one: i32 = -1;\n    minimum % minus_one\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("remainder compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "panic: attempt to calculate the remainder with overflow at <test.sl>:4:5\n"
    );
}
