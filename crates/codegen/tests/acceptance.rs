//! End-to-end acceptance scenarios for generics (plan section 5.9).
//!
//! Each scenario is a complete program that runs at O0 and O2 through the real
//! front end, borrow checker, monomorphization, and code generation. The
//! scenarios are grouped by language area so a regression points at one domain.

#[path = "acceptance/borrowing.rs"]
mod borrowing;
#[path = "acceptance/codegen.rs"]
mod codegen;
#[path = "acceptance/first_class_functions.rs"]
mod first_class_functions;
#[path = "acceptance/functions.rs"]
mod functions;
#[path = "acceptance/methods.rs"]
mod methods;
#[path = "acceptance/structs.rs"]
mod structs;
mod support;
#[path = "acceptance/traits.rs"]
mod traits;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, run};

/// Compiles `source` at both optimization levels and asserts its exit status.
pub(crate) fn assert_runs(name: &str, source: &str, result: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new(name);
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(result),
            "{name} returned the wrong value at {opt_level:?}"
        );
    }
}
