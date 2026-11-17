//! `explain` coverage for the monomorphization codes.
//!
//! Every user-visible code must have long-form text that names the construct
//! and the action a user can take.

use std::process::Command;

#[test]
fn monomorphization_codes_explain_the_action() {
    for (code, fragment) in [
        ("E1002", "The grammar required a specific token"),
        ("E2051", "A generic parameter name must be unique"),
        ("E3038", "A generic parameter has no value"),
        ("E3041", "A type parameter is opaque inside a generic body"),
        ("E7001", "instance chain that reached it"),
        ("E7002", "selects the implementation"),
        ("E7003", "there is no caller to supply its type"),
        ("E7004", "Reduce the number of distinct instantiations"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sealc"))
            .arg("explain")
            .arg(code)
            .output()
            .expect("the compiler runs");
        assert!(
            output.status.success(),
            "{code} failed to explain: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(fragment),
            "{code} explanation is missing {fragment:?}: {text}"
        );
    }
}
