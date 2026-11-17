//! Command-line coverage for the language examples.
//!
//! `--emit=check` runs through type checking, borrow checking, and
//! monomorphization without writing an artifact, so it proves the examples
//! compile through the same pipeline the driver uses.

use std::path::PathBuf;
use std::process::Command;

const GENERIC_EXAMPLES: [&str; 5] = [
    "027_generic_functions.sl",
    "028_generic_structs.sl",
    "029_generic_enums.sl",
    "030_generic_traits.sl",
    "031_generic_recursion.sl",
];

#[test]
fn generic_examples_check_cleanly() {
    for name in GENERIC_EXAMPLES {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_sealc"))
            .arg("--emit=check")
            .arg(&path)
            .output()
            .expect("the compiler runs");
        assert!(
            output.status.success(),
            "{name} failed to check: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{name} reported a diagnostic: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
