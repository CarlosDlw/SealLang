//! Acceptance for the documented raw-pointer example.
//!
//! `examples/043_raw_pointers.sl` is the public surface of the feature: an
//! exclusive raw pointer into an array, pointer arithmetic, an overlapping
//! `copy`, and a null query, all inside an `unsafe` block. The program returns
//! zero only when every operation reaches the expected address.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, assert_no_thread_locals_or_constructors, compile, contains, run};

const RAW_POINTERS: &str = include_str!("../../../examples/043_raw_pointers.sl");

#[test]
fn the_raw_pointer_example_runs_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "raw-pointer-example-debug"),
        (OptLevel::O2, false, "raw-pointer-example-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            RAW_POINTERS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("the example failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(0),
            "the example produced the wrong result at {opt_level:?}"
        );
    }
}

#[test]
fn the_raw_pointer_example_emits_plain_assembly() {
    let output = TempOutput::new("raw-pointer-example-assembly");
    compile(
        RAW_POINTERS,
        output.path(),
        Emit::Assembly,
        OptLevel::O0,
        false,
    )
    .expect("the example emits assembly");
    let assembly = fs::read(output.path()).expect("the assembly is readable");

    assert!(
        contains(&assembly, b"seal.main"),
        "the example must emit its entry point"
    );
    assert_no_thread_locals_or_constructors(&String::from_utf8_lossy(&assembly));
}
