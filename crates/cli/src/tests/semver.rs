//! API compatibility tests at the CLI boundary.

use seal_api::{ApiItem, ApiModule, ApiSnapshot, Bump};

fn snapshot(signature: &str) -> ApiSnapshot {
    ApiSnapshot {
        version: 1,
        package: "demo".to_owned(),
        package_version: "1.0.0".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        modules: vec![ApiModule {
            path: "root".to_owned(),
            exposed: true,
            package: 0,
            docs: None,
        }],
        items: vec![ApiItem {
            name: "value".to_owned(),
            kind: "function".to_owned(),
            module: "root".to_owned(),
            canonical: None,
            visibility: "public".to_owned(),
            cfg: None,
            signature: Some(signature.to_owned()),
            fields: Vec::new(),
            variants: Vec::new(),
            docs: None,
            deprecated: None,
            layout: None,
            implementations: Vec::new(),
        }],
    }
}

#[test]
fn a_signature_change_is_breaking() {
    let diff = seal_api::check(
        &snapshot("func value() -> i32"),
        &snapshot("func value() -> i64"),
    )
    .expect("compatible snapshots");
    assert_eq!(diff.required_bump(), Bump::Major);
}

#[test]
fn identical_snapshots_need_a_patch() {
    let diff = seal_api::check(
        &snapshot("func value() -> i32"),
        &snapshot("func value() -> i32"),
    )
    .expect("compatible snapshots");
    assert!(diff.is_empty());
    assert_eq!(diff.required_bump(), Bump::Patch);
}
