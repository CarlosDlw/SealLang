//! Documentation rendering tests.
//!
//! The CLI end-to-end tests cover snapshot generation; these tests keep the
//! rendering contract independent from the compiler.

use seal_api::{ApiField, ApiItem, ApiModule, ApiSnapshot};

fn snapshot() -> ApiSnapshot {
    ApiSnapshot {
        version: 1,
        package: "demo".to_owned(),
        package_version: "1.0.0".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        modules: vec![ApiModule {
            path: "root".to_owned(),
            exposed: true,
            package: 0,
            docs: Some("The root module.".to_owned()),
        }],
        items: vec![ApiItem {
            name: "Point".to_owned(),
            kind: "struct".to_owned(),
            module: "root".to_owned(),
            visibility: "public".to_owned(),
            cfg: Some("target.os == \"linux\"".to_owned()),
            signature: None,
            fields: vec![ApiField {
                name: Some("x".to_owned()),
                ty: "i32".to_owned(),
                visibility: "public".to_owned(),
                docs: Some("The x coordinate.".to_owned()),
            }],
            variants: Vec::new(),
            docs: Some("A point.".to_owned()),
            layout: Some(seal_api::ApiLayout { size: 4, align: 4 }),
            implementations: Vec::new(),
        }],
    }
}

#[test]
fn documentation_contains_docs_cfg_and_layout() {
    let documentation = seal_doc::Documentation::from_snapshot(&snapshot());
    let json = seal_doc::render_json(&documentation).expect("JSON");
    assert!(json.contains("A point."));
    assert!(json.contains("target.os"));
    assert!(json.contains("\"size\": 4"));
    let html = seal_doc::render_html(&documentation);
    assert!(html.contains("The root module."));
    assert!(html.contains("The x coordinate."));
    assert!(html.contains("root.html"));
}

#[test]
fn documentation_json_is_deterministic() {
    let documentation = seal_doc::Documentation::from_snapshot(&snapshot());
    assert_eq!(
        seal_doc::render_json(&documentation).expect("JSON"),
        seal_doc::render_json(&documentation).expect("JSON")
    );
}
