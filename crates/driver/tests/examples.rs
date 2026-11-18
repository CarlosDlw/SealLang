//! Command-line coverage for the language examples.
//!
//! `check` runs through type checking, borrow checking, and monomorphization
//! without writing an artifact, so it proves the examples compile through the
//! same pipeline the driver uses.

use std::path::PathBuf;
use std::process::Command;

const GENERIC_EXAMPLES: [&str; 5] = [
    "027_generic_functions.sl",
    "028_generic_structs.sl",
    "029_generic_enums.sl",
    "030_generic_traits.sl",
    "031_generic_recursion.sl",
];

const TRAIT_OBJECT_EXAMPLES: [&str; 3] = [
    "032_trait_objects.sl",
    "033_trait_objects_supertraits.sl",
    "034_trait_objects_generic.sl",
];

const NUMERIC_CAST_EXAMPLES: [&str; 1] = ["035_numeric_casts.sl"];

fn check_examples(names: &[&str]) {
    for name in names {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name);
        let output = Command::new(env!("CARGO_BIN_EXE_sealc"))
            .arg("check")
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

#[test]
fn generic_examples_check_cleanly() {
    check_examples(&GENERIC_EXAMPLES);
}

#[test]
fn trait_object_examples_check_cleanly() {
    check_examples(&TRAIT_OBJECT_EXAMPLES);
}

#[test]
fn numeric_cast_examples_check_cleanly() {
    check_examples(&NUMERIC_CAST_EXAMPLES);
}
