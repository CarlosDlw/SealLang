//! `explain` coverage for the monomorphization codes.
//!
//! Every user-visible code must have long-form text that names the construct
//! and the action a user can take.

use std::process::Command;

#[test]
fn global_data_codes_explain_the_action() {
    let codes = [
        "E1012", "E1013", "E1014", "E2068", "E3056", "E3057", "E3058", "E3059", "E3060", "E3062",
        "E3063", "E3070", "E3071", "E3072", "E3073", "E3074", "E3075", "E4001", "E4002", "E4003",
        "E4004", "E4005", "E4006", "E4007", "E4008", "E4009", "E4010", "E4011", "E4012", "E4013",
        "E4014", "E4015", "E4016", "E4017", "E4018", "E4019", "E4020", "E4021", "E4022", "E4023",
        "E4024", "E4025", "E4026", "E4027", "E4030", "E4031", "E4032", "E5011", "E5012", "E5013",
        "E6004", "E7007",
    ];
    for code in codes {
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
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.starts_with(&format!("{code}: ")),
            "{code} did not explain itself: {text}"
        );
        assert!(text.len() > 100, "{code} explanation is too short: {text}");
    }
}

#[test]
fn monomorphization_codes_explain_the_action() {
    for (code, fragment) in [
        ("E1002", "The grammar required a specific token"),
        ("E2051", "A generic parameter name must be unique"),
        ("E2065", "needs a finite vtable"),
        ("E3010", "`dyn Trait`"),
        ("E3027", "On a trait object"),
        ("E3029", "shared trait object"),
        ("E3038", "A generic parameter has no value"),
        ("E3041", "A type parameter is opaque inside a generic body"),
        ("E3046", "needs exactly one"),
        ("E3047", "reference kind is preserved"),
        ("E3048", "primitive numeric types only"),
        ("E3049", "no numeric-bound trait"),
        ("E7001", "instance chain that reached it"),
        ("E7002", "selects the implementation"),
        ("E7003", "there is no caller to supply its type"),
        ("E7004", "Reduce the number of distinct instantiations"),
        ("E7005", "missing entry"),
        ("E7006", "vtable collection limit"),
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
