//! Acceptance for the documented opt-in `Copy` example.
//!
//! `examples/044_copy_optin.sl` is the public surface of the feature: a struct,
//! an enum, a conditional generic wrapper, a reference holder, and a raw
//! pointer holder, all copied through assignment, calls, patterns, loops,
//! arrays, statics, and the no-op `drop`. The program returns the documented
//! value, and a `Copy` type contributes no drop glue.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, assert_no_thread_locals_or_constructors, compile, run};

const COPY_OPTIN: &str = include_str!("../../../examples/044_copy_optin.sl");

#[test]
fn the_copy_optin_example_runs_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "copy-optin-example-debug"),
        (OptLevel::O2, false, "copy-optin-example-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            COPY_OPTIN,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("the example failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "the example produced the wrong result at {opt_level:?}"
        );
    }
}

#[test]
fn the_copy_optin_example_emits_no_drop_glue() {
    let output = TempOutput::new("copy-optin-example-assembly");
    compile(
        COPY_OPTIN,
        output.path(),
        Emit::Assembly,
        OptLevel::O0,
        false,
    )
    .expect("the example emits assembly");
    let assembly = fs::read_to_string(output.path()).expect("the assembly is readable");

    assert!(
        assembly.contains("seal.main"),
        "the example must emit its entry point"
    );
    assert!(
        !assembly.contains("seal.drop.glue."),
        "a Copy type must not emit drop glue:\n{assembly}"
    );
    assert!(
        !assembly.contains("seal.drop.noop"),
        "a Copy type must not emit a no-op glue:\n{assembly}"
    );
    assert_no_thread_locals_or_constructors(&assembly);
}
