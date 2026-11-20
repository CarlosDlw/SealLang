//! Acceptance for the documented libc FFI example.
//!
//! `examples/045_ffi_libc.sl` is the public surface of the feature: an
//! `extern "c"` block, a scalar call to `abs`, a variadic call to `snprintf`
//! with a `c"..."` format string, and an `unsafe` boundary. The program returns
//! 7 only when both calls and the C string produce the documented bytes.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, assert_no_thread_locals_or_constructors, compile, run};

const FFI_LIBC: &str = include_str!("../../../examples/045_ffi_libc.sl");

#[test]
fn the_ffi_libc_example_runs_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "ffi-libc-example-debug"),
        (OptLevel::O2, false, "ffi-libc-example-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            FFI_LIBC,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("the example failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(7),
            "the example produced the wrong result at {opt_level:?}"
        );
    }
}

#[test]
fn the_ffi_libc_example_emits_plain_assembly() {
    let output = TempOutput::new("ffi-libc-example-assembly");
    compile(FFI_LIBC, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("the example emits assembly");
    let assembly = String::from_utf8(fs::read(output.path()).expect("the assembly is readable"))
        .expect("assembly is UTF-8");

    assert!(
        assembly.contains("seal.main"),
        "the example must emit its entry point"
    );
    assert_no_thread_locals_or_constructors(&assembly);
}
