//! End-to-end tests for native library linking.
//!
//! A real C fixture is compiled into an archive with the host toolchain, linked
//! into a Seal executable, and executed. The tests cover the command-line
//! library flags, the source `link` directive, raw linker arguments, and the
//! diagnostics for invalid link configuration.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Output;

use seal_codegen::{CodegenError, Emit};
use seal_error_codes::E6005;
use seal_session::OptLevel;

mod support;

use support::cc::{Compiler, TempDirectory};
use support::{LinkOptions, TempOutput, compile_with_links, run_captured};

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
    compile_and_run_captured(source, links, tag, opt_level)
        .status
        .code()
        .expect("the program runs")
}

fn compile_and_run_captured(
    source: &str,
    links: LinkOptions,
    tag: &str,
    opt_level: OptLevel,
) -> Output {
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
    run_captured(output.path())
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

fn variadic_program() -> String {
    let mut source = String::from(
        "extern \"c\" {\n\
         \tfunc ffi_cstr_len(text: *raw u8) -> i32;\n\
         \tfunc ffi_sum_ints(count: i32, ...) -> i32;\n\
         \tfunc ffi_sum_doubles(count: i32, ...) -> f64;\n\
         \tfunc ffi_variadic_wide(count: i32, ...) -> i64;\n\
         \tfunc ffi_variadic_kinds(count: i32, ...) -> i32;\n\
         \tfunc ffi_variadic_first_char(count: i32, ...) -> i32;\n\
         \tfunc snprintf(buffer: *raw mut u8, size: usize, format: *raw u8, ...) -> c_int;\n\
         }\n\
         const GREETING: *raw u8 = c\"Z\";\n\
         static BANNER: *raw u8 = c\"Q\";\n",
    );
    source.push_str("func main() -> i32 {\n");
    source.push_str("  unsafe {\n");
    source.push_str("    if ffi_cstr_len(c\"hello\") != 5 { return 30; }\n");
    source.push_str("    if ffi_cstr_len(c\"\") != 0 { return 31; }\n");
    source.push_str("    if ffi_cstr_len(c\"é日\") != 5 { return 32; }\n");
    source.push_str("    if ffi_cstr_len(c\"a\\nb\") != 3 { return 33; }\n");
    source.push_str("    if ffi_sum_ints(0) != 0 { return 42; }\n");
    source.push_str("    if ffi_sum_ints(1, -5i32) != -5 { return 43; }\n");
    source.push_str("    if ffi_sum_ints(1, false) != 0 { return 44; }\n");
    source.push_str("    if ffi_sum_ints(1, true) != 1 { return 45; }\n");
    source.push_str(
        "    if ffi_sum_ints(8, 1i32, 2i32, 3i32, 4i32, 5i32, 6i32, 7i32, 8i32) != 36 { return 46; }\n",
    );
    source.push_str("    if ffi_sum_ints(4, 1i8, 2u16, true, 38) != 42 { return 34; }\n");
    source.push_str("    if ffi_sum_doubles(2, 40.5f32, 1.5f64) != 42.0 { return 35; }\n");
    source.push_str("    if ffi_variadic_kinds(3, 1i8, 2.5f32, 3i64) != 1 { return 48; }\n");
    source.push_str("    if ffi_variadic_wide(2, 40i64, 2i64) != 42i64 { return 36; }\n");
    source.push_str("    if ffi_variadic_first_char(1, c\"Z\") != 90 { return 37; }\n");
    source.push_str("    if ffi_variadic_first_char(1, GREETING) != 90 { return 38; }\n");
    source.push_str("    if ffi_variadic_first_char(1, BANNER) != 81 { return 47; }\n");
    source.push_str("    let mut buffer: [u8; 32] = [0u8; 32];\n");
    source.push_str(
        "    let written = snprintf(&mut buffer[0] as *raw mut u8, 32usize, c\"value=%d\", 7);\n",
    );
    source.push_str("    if written != 7 { return 39; }\n");
    source
        .push_str("    if buffer[0] != 118 || buffer[5] != 61 || buffer[6] != 55 { return 40; }\n");
    source.push_str("    if buffer[7] != 0 { return 41; }\n");
    source.push_str("  }\n");
    source.push_str("  42\n");
    source.push_str("}\n");
    source
}

#[test]
fn c_strings_and_variadic_arguments_cross_the_c_boundary() {
    let fixture = Fixture::new();
    for (opt_level, tag) in [
        (OptLevel::O0, "ffi-variadic-o0"),
        (OptLevel::O2, "ffi-variadic-o2"),
    ] {
        let code = compile_and_run(&variadic_program(), fixture.links(), tag, opt_level);
        assert_eq!(code, 42, "variadic ABI failed at {opt_level:?}");
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

fn callback_program() -> String {
    String::from(
        r#"
        extern struct Point { x: c_int, y: c_int }
        extern "c" {
          func ffi_apply(callback: *raw extern "c" func(i32) -> i32, value: i32) -> i32;
          func ffi_scalar_i32(value: i32) -> i32;
          func ffi_apply_null(callback: *raw extern "c" func(i32) -> i32) -> i32;
          func ffi_signal_like(callback: *raw extern "c" func(i32) -> i32) -> i32;
          func ffi_apply_point(callback: *raw extern "c" func(Point) -> Point, point: Point) -> Point;
          func ffi_sum_ints(count: i32, ...) -> i32;
          func ffi_register_atexit(callback: *raw extern "c" func()) -> c_int;
          func ffi_exit_now(code: c_int) -> ();
          func qsort(base: *raw mut c_void, count: usize, size: usize, compare: *raw extern "c" func(*raw c_void, *raw c_void) -> c_int) -> ();
        }

        func double_it(value: i32) -> i32 { value * 2 }

        func add_point(point: Point) -> Point {
          Point { x: point.x + 1, y: point.y + 1 }
        }

        func compare_ints(left: *raw c_void, right: *raw c_void) -> c_int {
          unsafe {
            let a = *(left as *raw i32);
            let b = *(right as *raw i32);
            if a < b { -1 } else if a > b { 1 } else { 0 }
          }
        }

        func on_exit() {
          unsafe { ffi_exit_now(77); }
        }

        func main() -> i32 {
          let mut values = [3, 1, 2];
          unsafe {
            let callback = double_it as *raw extern "c" func(i32) -> i32;
            if ffi_apply(callback, 21) != 42 { return 1; }
            let external_callback = ffi_scalar_i32 as *raw extern "c" func(i32) -> i32;
            if ffi_apply(external_callback, 41) != 42 { return 14; }
            let null = 0usize as *raw extern "c" func(i32) -> i32;
            if !null.is_null() { return 13; }
            if ffi_apply_null(null) != 1 { return 2; }
            if ffi_apply_null(callback) != 0 { return 12; }
            if ffi_signal_like(callback) != 84 { return 3; }

            let compare = compare_ints as *raw extern "c" func(*raw c_void, *raw c_void) -> c_int;
            let base = &mut values[0] as *raw mut i32 as *raw mut c_void;
            qsort(base, 3usize, 4usize, compare);
            if values[0] != 1 { return 4; }
            if values[1] != 2 { return 5; }
            if values[2] != 3 { return 6; }

            let point_callback = add_point as *raw extern "c" func(Point) -> Point;
            let point = Point { x: 10, y: 20 };
            let moved = ffi_apply_point(point_callback, point);
            if moved.x != 11 { return 8; }
            if moved.y != 21 { return 9; }

            let variadic = ffi_sum_ints as *raw extern "c" func(i32, ...) -> i32;
            if variadic(3, 1i32, 2i32, 39i32) != 42 { return 10; }

            let exit_callback = on_exit as *raw extern "c" func();
            if ffi_register_atexit(exit_callback) != 0 { return 11; }
          }
          0
        }
    "#,
    )
}

#[test]
fn c_function_pointers_and_callbacks_cross_the_boundary() {
    let fixture = Fixture::new();
    for (opt_level, tag) in [
        (OptLevel::O0, "ffi-callback-o0"),
        (OptLevel::O2, "ffi-callback-o2"),
    ] {
        let code = compile_and_run(&callback_program(), fixture.links(), tag, opt_level);
        assert_eq!(code, 77, "the atexit callback ran (O{opt_level:?})");
    }
}

fn buffer_program() -> String {
    String::from(
        r#"
        extern "c" {
          func ffi_fill_buffer(out: *raw mut u8, cap: usize) -> ();
          func ffi_utf8_message() -> *raw u8;
          func seal_print(data: *raw u8, length: usize) -> c_int;
        }

        func main() -> i32 {
          let mut buffer = [0u8; 8];
          unsafe {
            let base = &mut buffer[0] as *raw mut u8;
            ffi_fill_buffer(base, 8usize);

            let slice = base.with_len(5);
            if slice.len() != 5 { return 1; }
            if slice.data().addr() != slice.addr() { return 2; }
            if (*slice)[0] != 104 { return 3; }
            if (*slice)[1] != 101 { return 4; }
            if (*slice)[4] != 111 { return 5; }

            let text = slice.as_str();
            if text.len() != 5 { return 6; }
            if text.data().addr() != slice.data().addr() { return 7; }
            if seal_print(text.data() as *raw u8, text.len()) != 0 { return 8; }

            let message = ffi_utf8_message();
            let message_text = message.with_len(4).as_str();
            if seal_print(c" ", 1usize) != 0 { return 9; }
            if seal_print(message_text.data() as *raw u8, message_text.len()) != 0 { return 10; }
          }
          42
        }
    "#,
    )
}

#[test]
fn c_buffers_become_slices_and_strings() {
    let fixture = Fixture::new();
    for (opt_level, tag) in [
        (OptLevel::O0, "ffi-buffer-o0"),
        (OptLevel::O2, "ffi-buffer-o2"),
    ] {
        let output = compile_and_run_captured(&buffer_program(), fixture.links(), tag, opt_level);
        assert_eq!(
            output.status.code(),
            Some(42),
            "wrong status at {opt_level:?}"
        );
        assert_eq!(
            output.stdout,
            b"hello ol\xc3\xa1".to_vec(),
            "wrong output at {opt_level:?}"
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
