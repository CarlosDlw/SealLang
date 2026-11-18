//! End-to-end coverage for the documented trait default-method example.

mod support;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, run};

const SOURCE: &str = include_str!("../../../examples/036_default_methods.sl");

#[test]
fn the_default_methods_example_produces_its_documented_result_at_o0_and_o2() {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("default-methods-example");
        compile(SOURCE, output.path(), Emit::Executable, opt_level, false)
            .expect("the default-methods example compiles");
        assert_eq!(
            run(output.path()).code(),
            Some(42),
            "wrong result at {opt_level:?}"
        );
    }
}
