//! Acceptance for the documented owned-view example.
//!
//! `examples/053_owned_views.sl` is the public surface of the feature: thin,
//! slice, and string views; shared and exclusive owners; a promoted temporary
//! owner; and a static owner. The program returns zero only when every view
//! reaches the expected storage and the borrow checker accepts the sequence.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, assert_no_thread_locals_or_constructors, compile, contains, run};

const OWNED_VIEWS: &str = include_str!("../../../examples/053_owned_views.sl");

#[test]
fn the_owned_view_example_runs_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "owned-view-example-debug"),
        (OptLevel::O2, false, "owned-view-example-release"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            OWNED_VIEWS,
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
fn the_owned_view_example_emits_plain_assembly() {
    let output = TempOutput::new("owned-view-example-assembly");
    compile(
        OWNED_VIEWS,
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
