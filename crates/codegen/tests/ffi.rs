//! End-to-end tests for native library linking.
//!
//! A real C fixture is compiled into an archive with the host toolchain, linked
//! into a Seal executable, and executed. The tests cover the command-line
//! library flags, the source `link` directive, raw linker arguments, and the
//! diagnostics for invalid link configuration.

use std::ffi::OsString;
use std::path::PathBuf;

use seal_codegen::{CodegenError, Emit};
use seal_error_codes::E6005;
use seal_session::OptLevel;

mod support;

use support::cc::{Compiler, TempDirectory};
use support::{LinkOptions, TempOutput, compile_with_links, run};

/// The C fixture archive kept alive for one test.
struct Fixture {
    temp: TempDirectory,
    provider: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDirectory::new("ffi");
        let compiler = Compiler::discover();
        let extension = if compiler.is_msvc() { "obj" } else { "o" };
        let object = temp.path().join(format!("ffi_fixture.{extension}"));
        let archive = temp.path().join(if compiler.is_msvc() {
            "sealfixture.lib"
        } else {
            "libsealfixture.a"
        });
        compiler.compile_object(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/ffi_fixture.c"),
            None,
            &object,
        );
        compiler.archive(&object, &archive);

        let provider = temp.path().join(format!("ffi_external.{extension}"));
        compiler.compile_object(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/ffi_link_arg.c"),
            None,
            &provider,
        );

        Self { temp, provider }
    }

    fn links(&self) -> LinkOptions {
        LinkOptions {
            libraries: vec!["sealfixture".to_owned()],
            search_paths: vec![self.temp.path().to_path_buf()],
            link_args: Vec::new(),
        }
    }
}

/// Declarations shared by the fixture programs.
fn declarations(with_directive: bool, extra: &str) -> String {
    let mut source = String::from("extern \"c\" {\n");
    if with_directive {
        source.push_str("  link \"sealfixture\";\n");
    }
    source.push_str("  func ffi_scalar_i32(value: i32) -> i32;\n");
    source.push_str("  func ffi_scalar_u64(value: u64) -> u64;\n");
    source.push_str("  func ffi_scalar_f64(value: f64) -> f64;\n");
    source.push_str("  func ffi_ptr_roundtrip(pointer: *raw u8) -> *raw u8;\n");
    source.push_str("  func ffi_set_int(value: i32);\n");
    source.push_str("  func ffi_get_int() -> i32;\n");
    source.push_str("  func ffi_null_is_null(pointer: *raw u8) -> i32;\n");
    source.push_str("  func ffi_char_signed() -> i32;\n");
    source.push_str("  func ffi_long_width() -> i32;\n");
    source.push_str(extra);
    source.push_str("}\n");
    source
}

fn scalar_program(with_directive: bool) -> String {
    let mut source = declarations(with_directive, "");
    source.push_str("func main() -> i32 {\n");
    source.push_str("  unsafe {\n");
    source.push_str("    if ffi_scalar_i32(41) != 42 { return 1; }\n");
    source.push_str("    if ffi_scalar_u64(21u64) != 42u64 { return 2; }\n");
    source.push_str("    if ffi_scalar_f64(41.5) != 42.0 { return 3; }\n");
    source.push_str("    let value = 7;\n");
    source.push_str("    let pointer = &value as *raw i32 as *raw u8;\n");
    source
        .push_str("    if ffi_ptr_roundtrip(pointer) as usize != pointer as usize { return 4; }\n");
    source.push_str("    ffi_set_int(42);\n");
    source.push_str("    if ffi_get_int() != 42 { return 5; }\n");
    source.push_str("    if ffi_null_is_null(0usize as *raw u8) != 1 { return 6; }\n");
    source.push_str("  }\n");
    source.push_str("  42\n");
    source.push_str("}\n");
    source
}

fn compile_and_run(source: &str, links: LinkOptions, tag: &str, opt_level: OptLevel) -> i32 {
    let output = TempOutput::new(tag);
    compile_with_links(
        source,
        output.path(),
        Emit::Executable,
        opt_level,
        opt_level == OptLevel::O0,
        links,
    )
    .expect("the fixture program links");
    run(output.path()).code().expect("the program runs")
}

#[test]
fn fixture_scalars_roundtrip_through_a_cli_library() {
    let fixture = Fixture::new();
    for (opt_level, tag) in [(OptLevel::O0, "ffi-o0"), (OptLevel::O2, "ffi-o2")] {
        let code = compile_and_run(&scalar_program(false), fixture.links(), tag, opt_level);
        assert_eq!(code, 42, "wrong result at {opt_level:?}");
    }
}

fn aggregate_program() -> String {
    let mut source = String::from(
        "extern type Handle;\n\
         extern struct Point { x: c_int, y: c_int }\n\
         extern struct Nested { point: Point, scale: f64 }\n\
         extern struct Array { values: [c_int; 3] }\n\
         extern struct Mixed { small: i8, wide: i64, real: f64 }\n\
         extern struct Wide { first: c_int, second: f64, point: Point, last: i64 }\n\
         extern \"c\" {\n",
    );
    source.push_str("  func ffi_point_new(x: c_int, y: c_int) -> Point;\n");
    source.push_str("  func ffi_point_sum(point: Point) -> c_int;\n");
    source.push_str("  func ffi_point_make(x: c_int, y: c_int) -> Point;\n");
    source.push_str("  func ffi_nested_make(x: c_int, y: c_int, scale: f64) -> Nested;\n");
    source.push_str("  func ffi_nested_sum(nested: Nested) -> f64;\n");
    source.push_str("  func ffi_array_make(first: c_int, second: c_int, third: c_int) -> Array;\n");
    source.push_str("  func ffi_array_sum(array: Array) -> c_int;\n");
    source.push_str("  func ffi_mixed_make(small: i8, wide: i64, real: f64) -> Mixed;\n");
    source.push_str("  func ffi_mixed_check(mixed: Mixed) -> c_int;\n");
    source.push_str(
        "  func ffi_wide_make(first: c_int, second: f64, x: c_int, y: c_int, last: i64) -> Wide;\n",
    );
    source.push_str("  func ffi_wide_check(wide: Wide) -> c_int;\n");
    source.push_str("  func ffi_sizeof_point() -> c_int;\n");
    source.push_str("  func ffi_offsetof_point_y() -> c_int;\n");
    source.push_str("  func ffi_sizeof_nested() -> c_int;\n");
    source.push_str("  func ffi_sizeof_array() -> c_int;\n");
    source.push_str("  func ffi_sizeof_mixed() -> c_int;\n");
    source.push_str("  func ffi_sizeof_wide() -> c_int;\n");
    source.push_str("  func ffi_handle_new(value: c_int) -> *raw mut Handle;\n");
    source.push_str("  func ffi_handle_get(handle: *raw mut Handle) -> c_int;\n");
    source.push_str("  func ffi_handle_set(handle: *raw mut Handle, value: c_int);\n");
    source.push_str("  func ffi_handle_free(handle: *raw mut Handle);\n");
    source.push_str("}\n");
    source.push_str("func main() -> i32 {\n");
    source.push_str("  unsafe {\n");
    source.push_str("    let point = ffi_point_new(40, 2);\n");
    source.push_str("    if point.x != 40 || point.y != 2 { return 10; }\n");
    source.push_str("    if ffi_point_sum(point) != 42 { return 11; }\n");
    source.push_str("    let made = ffi_point_make(20, 22);\n");
    source.push_str("    if made.x + made.y != 42 { return 12; }\n");
    source.push_str("    let nested = ffi_nested_make(1, 2, 3.0);\n");
    source.push_str("    if nested.point.x != 1 || nested.point.y != 2 { return 13; }\n");
    source.push_str("    if ffi_nested_sum(nested) != 9.0 { return 14; }\n");
    source.push_str("    let array = ffi_array_make(1, 2, 39);\n");
    source.push_str("    if array.values[0] != 1 { return 16; }\n");
    source.push_str("    if ffi_array_sum(array) != 42 { return 15; }\n");
    source.push_str("    let mixed = ffi_mixed_make(7i8, 35i64, 2.5);\n");
    source.push_str("    if ffi_mixed_check(mixed) != 1 { return 17; }\n");
    source.push_str("    let wide = ffi_wide_make(1, 2.5, 3, 4, 5i64);\n");
    source.push_str("    if ffi_wide_check(wide) != 1 { return 18; }\n");
    source.push_str("    if ffi_sizeof_point() != 8 { return 19; }\n");
    source.push_str("    if ffi_offsetof_point_y() != 4 { return 20; }\n");
    source.push_str("    if ffi_sizeof_nested() != 16 { return 21; }\n");
    source.push_str("    if ffi_sizeof_array() != 12 { return 22; }\n");
    source.push_str("    if ffi_sizeof_mixed() != 24 { return 23; }\n");
    source.push_str("    if ffi_sizeof_wide() != 32 { return 24; }\n");
    source.push_str("    let handle = ffi_handle_new(41);\n");
    source.push_str("    if ffi_handle_get(handle) != 41 { return 25; }\n");
    source.push_str("    ffi_handle_set(handle, 42);\n");
    source.push_str("    if ffi_handle_get(handle) != 42 { return 26; }\n");
    source.push_str("    ffi_handle_free(handle);\n");
    source.push_str("  }\n");
    source.push_str("  42\n");
    source.push_str("}\n");
    source
}

#[test]
fn extern_structs_and_opaque_handles_roundtrip_by_value() {
    let fixture = Fixture::new();
    for (opt_level, tag) in [
        (OptLevel::O0, "ffi-aggregate-o0"),
        (OptLevel::O2, "ffi-aggregate-o2"),
    ] {
        let code = compile_and_run(&aggregate_program(), fixture.links(), tag, opt_level);
        assert_eq!(code, 42, "aggregate ABI failed at {opt_level:?}");
    }
}

#[test]
fn a_source_link_directive_names_the_library() {
    let fixture = Fixture::new();
    let links = LinkOptions {
        libraries: Vec::new(),
        search_paths: vec![fixture.temp.path().to_path_buf()],
        link_args: Vec::new(),
    };
    let code = compile_and_run(&scalar_program(true), links, "ffi-directive", OptLevel::O0);
    assert_eq!(code, 42);
}

#[test]
fn the_fixture_agrees_with_the_target_c_abi() {
    let fixture = Fixture::new();
    let abi = seal_target::host_triple().spec().c_abi();
    let mut source = declarations(false, "");
    source.push_str("func main() -> i32 {\n");
    source.push_str("  unsafe { ffi_char_signed() * 100 + ffi_long_width() }\n");
    source.push_str("}\n");
    let code = compile_and_run(&source, fixture.links(), "ffi-abi", OptLevel::O0);
    let signed = if abi.char_signed() { 1 } else { 0 };
    let expected = signed * 100 + i32::from(abi.long_width());
    assert_eq!(
        code, expected,
        "C char signedness and long width must match CAbiProperties"
    );
}

#[test]
fn link_arguments_link_an_extra_object() {
    let fixture = Fixture::new();
    let source = "\
        extern \"c\" { func ffi_link_arg_value() -> i32; }\n\
        func main() -> i32 { unsafe { ffi_link_arg_value() } }";
    let links = LinkOptions {
        libraries: Vec::new(),
        search_paths: Vec::new(),
        link_args: vec![OsString::from(fixture.provider.as_os_str())],
    };
    let code = compile_and_run(source, links, "ffi-link-arg", OptLevel::O0);
    assert_eq!(code, 42);
}

#[test]
fn a_missing_library_fails_with_e6003_and_names_the_library() {
    let fixture = Fixture::new();
    let mut links = fixture.links();
    links.libraries = vec!["seal_missing_fixture".to_owned()];
    let output = TempOutput::new("ffi-missing-library");
    let error = compile_with_links(
        "func main() -> i32 { 0 }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        links,
    )
    .expect_err("a missing library cannot link");
    match error {
        CodegenError::Artifact { detail, .. } => {
            assert!(
                detail.contains("seal_missing_fixture"),
                "the diagnostic names the library: {detail}"
            );
            assert!(
                detail.contains("command:"),
                "the diagnostic includes the command: {detail}"
            );
        }
        other => panic!("expected E6003, got {other:?}"),
    }
}

#[test]
fn invalid_library_names_are_e6005() {
    let fixture = Fixture::new();
    for name in ["", "a/b", r"a\b", "a b"] {
        let mut links = fixture.links();
        links.libraries = vec![name.to_owned()];
        let output = TempOutput::new("ffi-invalid-library");
        let error = compile_with_links(
            "func main() -> i32 { 0 }",
            output.path(),
            Emit::Executable,
            OptLevel::O0,
            false,
            links,
        )
        .expect_err("an invalid name cannot link");
        assert_eq!(
            error.diag_code(),
            Some(E6005.diag_code()),
            "`{name}` must be E6005: {error}"
        );
        let message = error.to_string();
        if name.is_empty() {
            assert!(message.contains("cannot be empty"), "{message}");
        } else {
            assert!(
                message.contains(name),
                "the message names `{name}`: {message}"
            );
        }
        assert!(
            !output.path().exists(),
            "no artifact is written for an invalid name"
        );
    }
}

#[test]
fn a_missing_search_path_is_e6005() {
    let fixture = Fixture::new();
    let mut links = fixture.links();
    links.search_paths = vec![fixture.temp.path().join("missing")];
    let output = TempOutput::new("ffi-missing-path");
    let error = compile_with_links(
        "func main() -> i32 { 0 }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        links,
    )
    .expect_err("a missing search path cannot link");
    assert_eq!(error.diag_code(), Some(E6005.diag_code()));
    let message = error.to_string();
    assert!(
        message.contains("does not exist") && message.contains("missing"),
        "the message names the missing path: {message}"
    );
    assert!(!output.path().exists(), "no artifact is written");
}

#[test]
fn a_search_path_that_is_a_file_is_e6005() {
    let fixture = Fixture::new();
    let mut links = fixture.links();
    links.search_paths = vec![fixture.provider.clone()];
    let output = TempOutput::new("ffi-file-path");
    let error = compile_with_links(
        "func main() -> i32 { 0 }",
        output.path(),
        Emit::Executable,
        OptLevel::O0,
        false,
        links,
    )
    .expect_err("a file is not a search directory");
    assert_eq!(error.diag_code(), Some(E6005.diag_code()));
    assert!(
        error.to_string().contains("is not a directory"),
        "the message explains the path kind: {error}"
    );
}

#[test]
fn object_and_assembly_output_ignore_link_inputs() {
    let fixture = Fixture::new();
    for (emit, tag) in [(Emit::Object, "ffi-object"), (Emit::Assembly, "ffi-asm")] {
        let output = TempOutput::new(tag);
        compile_with_links(
            "func main() -> i32 { 0 }",
            output.path(),
            emit,
            OptLevel::O0,
            false,
            LinkOptions {
                libraries: vec!["a/b".to_owned()],
                search_paths: vec![fixture.temp.path().join("missing")],
                link_args: Vec::new(),
            },
        )
        .expect("object and assembly output do not link");
        assert!(output.path().exists());
    }
}
