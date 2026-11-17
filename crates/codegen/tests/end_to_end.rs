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
const ARRAYS: &str = include_str!("../../../examples/009_arrays.sl");
const SLICES: &str = include_str!("../../../examples/010_slices.sl");
const METHODS: &str = include_str!("../../../examples/011_methods.sl");
const USER_METHODS: &str = include_str!("../../../examples/012_user_methods.sl");
const METHOD_BORROWS: &str = include_str!("../../../examples/013_method_borrows.sl");
const METHOD_NAMESPACES: &str = include_str!("../../../examples/014_method_namespaces.sl");
const TRAITS: &str = include_str!("../../../examples/015_traits.sl");
const TRAIT_RECEIVERS: &str = include_str!("../../../examples/016_trait_receivers.sl");
const ENUMS: &str = include_str!("../../../examples/017_enums.sl");
const TUPLE_VARIANTS: &str = include_str!("../../../examples/018_tuple_variants.sl");

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
    let aliasing = seal_borrowck::check(&hir, &mir, session.diag(), session.interner());
    assert!(!session.diag().has_errors(), "borrow checking failed");

    seal_codegen::emit(
        CodegenInput {
            mir: &mir,
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

#[test]
fn arrays_work_in_debug_and_release_builds() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "arrays-debug"),
        (OptLevel::O2, false, "arrays-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            ARRAYS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("array example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(71),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn slices_coerce_range_index_and_measure_length() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "debug"),
        (OptLevel::O2, false, "release"),
    ] {
        let output = TempOutput::new(&format!("slices-{suffix}"));
        compile(
            SLICES,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("slice example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(120),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn methods_produce_the_language_result_in_debug_and_release() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "methods-debug"),
        (OptLevel::O2, false, "methods-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            METHODS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("method example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(89),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn user_methods_produce_the_language_result_in_debug_and_release() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "user-methods-debug"),
        (OptLevel::O2, false, "user-methods-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            USER_METHODS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("the user method example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(47),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn method_receiver_borrows_follow_the_borrow_checker() {
    let output = TempOutput::new("method-borrows");
    compile(
        METHOD_BORROWS,
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the method borrow example compiles");
    assert_eq!(run(output.path()).code(), Some(52));
}

#[test]
fn same_named_methods_on_different_types_get_distinct_symbols() {
    let output = TempOutput::new("method-namespaces");
    compile(
        METHOD_NAMESPACES,
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("same-named methods on different types compile");
    assert_eq!(run(output.path()).code(), Some(16));
}

#[test]
fn traits_dispatch_statically_in_debug_and_release() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "traits-debug"),
        (OptLevel::O2, false, "traits-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            TRAITS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("the trait example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(52),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_mutable_receiver_reborrows_shared_for_trait_methods() {
    let output = TempOutput::new("trait-receivers");
    compile(
        TRAIT_RECEIVERS,
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the trait receiver example compiles");
    assert_eq!(run(output.path()).code(), Some(42));
}

#[test]
fn enums_construct_and_move_in_debug_and_release() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "enums-debug"),
        (OptLevel::O2, false, "enums-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            ENUMS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("the enum example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn tuple_variants_construct_and_move_in_debug_and_release() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "tuple-variants-debug"),
        (OptLevel::O2, false, "tuple-variants-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            TUPLE_VARIANTS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("the tuple variant example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_method_named_main_is_not_the_entry_point() {
    let output = TempOutput::new("method-main-missing");
    let inherent = compile(
        "struct App { value: i32 }\n\
         impl App { func main(self: *App) -> i32 { (*self).value } }\n",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect_err("an inherent method is not an entry point");
    assert!(matches!(
        inherent,
        CodegenError::EntryPoint(EntryPointError::Missing)
    ));

    let trait_method = compile(
        "struct App { value: i32 }\n\
         trait Run { func main(self: *Self) -> i32; }\n\
         impl Run for App { func main(self: *App) -> i32 { (*self).value } }\n",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
    )
    .expect_err("a trait method is not an entry point");
    assert!(matches!(
        trait_method,
        CodegenError::EntryPoint(EntryPointError::Missing)
    ));
}

#[test]
fn a_free_main_takes_precedence_over_a_method_named_main() {
    let output = TempOutput::new("method-main-free");
    compile(
        "struct App { value: i32 }\n\
         impl App { func main(self: *App) -> i32 { (*self).value } }\n\
         func main() -> i32 { let app = App { value: 7 }; app.main() + 35 }\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the free function is the entry point");
    assert_eq!(run(output.path()).code(), Some(42));
}

#[test]
fn a_trait_method_with_a_value_receiver_moves_the_value() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "trait-value-debug"),
        (OptLevel::O2, false, "trait-value-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Token { value: i32 }\n\
             trait Consume { func consume(self: Self) -> i32; }\n\
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } }\n\
             func main() -> i32 { let token = Token { value: 42 }; token.consume() }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("a value receiver compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_value_receiver_accepts_a_temporary_receiver() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "trait-value-temporary-debug"),
        (OptLevel::O2, false, "trait-value-temporary-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Token { value: i32 }\n\
             trait Consume { func consume(self: Self) -> i32; }\n\
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } }\n\
             func main() -> i32 { Token { value: 21 }.consume() * 2 }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("a temporary receiver compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_value_receiver_moves_a_field_out_of_a_struct() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "trait-value-field-debug"),
        (OptLevel::O2, false, "trait-value-field-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Token { value: i32 }\n\
             struct Holder { token: Token }\n\
             trait Consume { func consume(self: Self) -> i32; }\n\
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } }\n\
             func main() -> i32 {\n\
               let token = Token { value: 21 };\n\
               let holder = Holder { token: token };\n\
               holder.token.consume() * 2\n\
             }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("a field receiver compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_value_receiver_moves_an_element_out_of_an_array() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "trait-value-array-debug"),
        (OptLevel::O2, false, "trait-value-array-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Token { value: i32 }\n\
             trait Consume { func consume(self: Self) -> i32; }\n\
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } }\n\
             func main() -> i32 {\n\
               let items = [Token { value: 21 }, Token { value: 7 }];\n\
               let first = items[0].consume();\n\
               first + items[1].value\n\
             }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("an array element receiver compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(28),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn sibling_fields_stay_usable_after_moving_a_trait_receiver_field() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "trait-value-sibling-debug"),
        (OptLevel::O2, false, "trait-value-sibling-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Token { value: i32 }\n\
             struct Holder { token: Token, bonus: i32 }\n\
             trait Consume { func consume(self: Self) -> i32; }\n\
             impl Consume for Token { func consume(self: Token) -> i32 { self.value } }\n\
             func main() -> i32 {\n\
               let holder = Holder { token: Token { value: 21 }, bonus: 21 };\n\
               holder.token.consume() + holder.bonus\n\
             }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("a sibling field stays usable");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn a_shared_receiver_borrows_through_a_slice_and_an_explicit_dereference() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "receiver-auto-borrow-debug"),
        (OptLevel::O2, false, "receiver-auto-borrow-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Counter { value: i32 }\n\
             trait ReadValue { func read(self: *Self) -> i32; }\n\
             impl ReadValue for Counter { func read(self: *Counter) -> i32 { (*self).value } }\n\
             func first(counters: *[Counter; 1]) -> i32 { counters[0].read() }\n\
             func peek(counter: *mut Counter) -> i32 { (*counter).read() }\n\
             func main() -> i32 {\n\
               let counters = [Counter { value: 21 }];\n\
               let mut counter = Counter { value: 21 };\n\
               first(&counters) + peek(&mut counter)\n\
             }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("auto-borrowed receivers compile");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn shared_and_exclusive_receivers_borrow_mutable_array_elements() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "receiver-mutable-element-debug"),
        (OptLevel::O2, false, "receiver-mutable-element-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            "struct Counter { value: i32 }\n\
             trait ReadValue { func read(self: *Self) -> i32; }\n\
             trait Bump { func bump(self: *mut Self); }\n\
             impl ReadValue for Counter { func read(self: *Counter) -> i32 { (*self).value } }\n\
             impl Bump for Counter { func bump(self: *mut Counter) { (*self).value += 1; } }\n\
             func peek(counters: *mut [Counter; 1]) -> i32 {\n\
               counters[0].bump();\n\
               counters[0].read()\n\
             }\n\
             func main() -> i32 {\n\
               let mut counters = [Counter { value: 20 }];\n\
               peek(&mut counters) * 2\n\
             }\n",
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("mutable element receivers compile");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn swap_reverse_and_fill_handle_non_copy_elements() {
    let source = "struct Point { x: i32 }\n\
                  func main() -> i32 {\n\
                    let mut points = [Point { x: 1 }, Point { x: 2 }, Point { x: 3 }];\n\
                    points.reverse();\n\
                    points.swap(0, 2);\n\
                    points[0].x + points[1].x + points[2].x\n\
                  }\n";
    let output = TempOutput::new("methods-non-copy");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("non-Copy sequence methods compile");
    assert_eq!(run(output.path()).code(), Some(6));
}

#[test]
fn empty_sequences_accept_reverse_and_fill() {
    let source = "func main() -> i32 {\n    let mut values: [i32; 0] = [];\n    values.reverse();\n    values.fill(9);\n    if values.len() == 0usize { 0 } else { 1 }\n}\n";
    let output = TempOutput::new("methods-empty");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("empty sequence methods compile");
    assert_eq!(run(output.path()).code(), Some(0));
}

#[test]
fn swap_checks_both_indices_before_writing() {
    let output = TempOutput::new("methods-swap-bounds");
    compile(
        "func main() {\n    let mut values = [1, 2, 3];\n    let index: usize = 3;\n    values.swap(0, index);\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the swap bounds check compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("index out of bounds"), "{stderr}");
    assert!(stderr.contains("at <test.sl>:4:5"), "{stderr}");
}

#[test]
fn clamp_reports_an_inverted_range() {
    let output = TempOutput::new("methods-clamp-bounds");
    compile(
        "func main() -> i32 { 1.clamp(5, 2) }",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the clamp check compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("minimum <= maximum"), "{stderr}");
}

#[test]
fn toggle_through_an_exclusive_reference_writes_the_pointee() {
    let source = "func main() -> i32 {\n\
                    let mut flag = false;\n\
                    let reference: *mut bool = &mut flag;\n\
                    reference.toggle();\n\
                    if flag { 42 } else { 7 }\n\
                  }\n";
    let output = TempOutput::new("methods-toggle-reference");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("toggle through a reference compiles");
    assert_eq!(run(output.path()).code(), Some(42));
}

#[test]
fn saturating_i128_and_nan_aware_float_methods_run() {
    let source = "func main() -> i32 {\n\
                    let maximum: i128 = 170141183460469231731687303715884105727i128;\n\
                    let nan = 0.0 / 0.0;\n\
                    if maximum.saturating_add(1i128) == maximum\n\
                       && nan.min(3.0) == 3.0\n\
                       && 3.0.max(nan) == 3.0 { 0 } else { 1 }\n\
                  }\n";
    let output = TempOutput::new("methods-edge-values");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("edge-value methods compile");
    assert_eq!(run(output.path()).code(), Some(0));
}

#[test]
fn slices_of_structs_keep_their_element_layout() {
    let output = TempOutput::new("slice-structs");
    compile(
        "struct Point { x: i32 }\n\
         func main() -> i32 {\n\
           let points = [Point { x: 3 }, Point { x: 4 }];\n\
           let slice = &points[..];\n\
           slice[1].x\n\
         }\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("a slice of structs compiles");
    assert_eq!(run(output.path()).code(), Some(4));
}

#[test]
fn a_slice_end_out_of_bounds_panics_before_the_access() {
    let output = TempOutput::new("slice-bounds");
    compile(
        "func main() -> i32 {\n\
           let values = [1, 2, 3];\n\
           let index: usize = 4;\n\
           let slice = &values[1..index];\n\
           slice[0]\n\
         }\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the range check compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("slice range end is out of bounds"),
        "{stderr}"
    );
    assert!(stderr.contains("at <test.sl>:4:13"), "{stderr}");
}

#[test]
fn a_dynamic_array_index_panics_before_the_access() {
    let output = TempOutput::new("array-bounds");
    compile(
        "func main() -> i32 {\n  let a = [1, 2, 3];\n  let i: usize = 3;\n  a[i]\n}\n",
        output.path(),
        Emit::Executable,
        OptLevel::O2,
        false,
    )
    .expect("the bounds check compiles");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("index out of bounds: the length is 3"),
        "{stderr}"
    );
    assert!(stderr.contains("at <test.sl>:4:5"), "{stderr}");
}

#[test]
fn arrays_of_structs_and_structs_with_array_fields_run() {
    let source = "struct Row { values: [i32; 2] }\n\
                  func main() -> i32 {\n\
                    let row = Row { values: [40, 1] };\n\
                    let rows = [row, Row { values: [0, 1] }];\n\
                    rows[0].values[0] + rows[0].values[1]\n\
                  }\n";
    let output = TempOutput::new("array-aggregates");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("aggregate arrays compile");
    assert_eq!(run(output.path()).code(), Some(41));
}

#[test]
fn zero_length_arrays_and_copy_assignments_run() {
    let source = "func main() -> i32 {\n\
                    let empty: [i32; 0] = [];\n\
                    let values = [1, 2];\n\
                    let copy = values;\n\
                    values[0] + copy[0]\n\
                  }\n";
    let output = TempOutput::new("array-empty-copy");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("empty arrays and copies compile");
    assert_eq!(run(output.path()).code(), Some(2));
}

#[test]
fn dynamic_array_writes_round_trip() {
    let source = "func main() -> i32 {\n\
                    let mut a = [1, 2, 3, 4];\n\
                    let i: usize = 1;\n\
                    let j: usize = 3;\n\
                    a[i] = 10;\n\
                    a[j] = a[i] + 5;\n\
                    a[0] + a[1] + a[2] + a[3]\n\
                  }\n";
    let output = TempOutput::new("array-writes");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("dynamic writes compile");
    assert_eq!(run(output.path()).code(), Some(29));
}

#[test]
fn a_repeated_array_inside_a_loop_allocates_its_temporary_once() {
    let source = "func main() -> i32 {\n\
                    let mut i = 0;\n\
                    let mut total = 0;\n\
                    while i < 1000 {\n\
                      let values = [1; 4];\n\
                      total += values[0];\n\
                      i += 1;\n\
                    }\n\
                    total\n\
                  }\n";
    let output = TempOutput::new("array-repeat-loop");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("repeat inside a loop compiles");
    assert_eq!(run(output.path()).code(), Some(232));
}

#[test]
fn arrays_pass_by_value_through_function_parameters() {
    let source = "func first(values: [i32; 3]) -> i32 { values[0] }\n\
                  func main() -> i32 { first([41, 1, 2]) + 1 }\n";
    let output = TempOutput::new("array-parameter");
    compile(source, output.path(), Emit::Executable, OptLevel::O2, false)
        .expect("array parameters compile");
    assert_eq!(run(output.path()).code(), Some(42));
}
