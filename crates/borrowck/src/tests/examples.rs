//! End-to-end borrow checking of the example programs.

use std::path::PathBuf;

use super::support::check_clean;

const EXAMPLES: [&str; 5] = [
    "0001_main.sl",
    "0002_variables.sl",
    "0003_operators.sl",
    "004_conditionals.sl",
    "005_while_loop.sl",
];

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn all_examples_borrow_check() {
    for name in EXAMPLES {
        check_clean(&source(name));
    }
}
