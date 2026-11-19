//! Acceptance for the documented destructor example.
//!
//! `examples/042_drop_destructors.sl` is the public surface of the feature: a
//! struct destructor, an enum, a generic type, early `drop(value)`, and a
//! by-value `for` with `break`. The program returns 0 only when every
//! destructor ran in the documented order, so the exit status is the assertion.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, assert_no_thread_locals_or_constructors, compile, run};

const DROP_DESTRUCTORS: &str = include_str!("../../../examples/042_drop_destructors.sl");

#[test]
fn the_destructor_example_runs_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "drop-example-debug"),
        (OptLevel::O2, false, "drop-example-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            DROP_DESTRUCTORS,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("the example failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(0),
            "the example produced the wrong destructor order at {opt_level:?}"
        );
    }
}

#[test]
fn the_example_emits_drop_glue_and_no_constructors() {
    let output = TempOutput::new("drop-example-asm");
    compile(
        DROP_DESTRUCTORS,
        output.path(),
        Emit::Assembly,
        OptLevel::O0,
        false,
    )
    .expect("the example emits assembly");
    let assembly = fs::read_to_string(output.path()).expect("the assembly is readable");
    assert!(
        assembly.contains("seal.drop.glue."),
        "the example must emit drop glue:\n{assembly}"
    );
    assert_no_thread_locals_or_constructors(&assembly);
}
