//! Metadata contract tests.

use crate::cli::Template;

use super::{fixture, load, write};

#[test]
fn metadata_lists_modules_files_and_targets() {
    let fixture = fixture("metadata");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"demo\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/main.sl"),
        "use app::util::value;\n\nfunc main() -> i32 { value() }\n",
    );
    write(
        &fixture.root.join("src/util.sl"),
        "module app::util;\n\ninternal func value() -> i32 { 1 }\n",
    );
    let project = load(&fixture);
    let text = crate::cli::metadata::report_json(&project, false).expect("metadata serializes");
    let value: serde_json::Value = serde_json::from_str(&text).expect("metadata is JSON");

    assert_eq!(value["version"], 1);
    assert_eq!(value["project"], "demo");
    assert_eq!(
        value["target"]["triple"],
        seal_target::host_triple().as_str()
    );
    let package = &value["packages"][0];
    assert_eq!(package["name"], "demo");
    assert_eq!(package["version"], "0.1.0");
    assert!(
        package["targets"]
            .as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["kind"] == "bin"))
    );
    assert!(
        package["modules"]
            .as_array()
            .is_some_and(|modules| modules.iter().any(|module| module == "app::util"))
    );
    assert!(
        package["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file == "src/main.sl"))
    );
}

#[test]
fn no_deps_keeps_only_the_default_package_in_a_workspace() {
    let fixture = fixture("metadata-workspace");
    crate::cli::new::scaffold(&fixture.root, "demo", Template::Workspace)
        .expect("the workspace scaffold is written");
    let project = load(&fixture);

    let full: serde_json::Value = serde_json::from_str(
        &crate::cli::metadata::report_json(&project, false).expect("metadata"),
    )
    .expect("metadata is JSON");
    assert_eq!(full["packages"].as_array().map(Vec::len), Some(2));

    let default_only: serde_json::Value =
        serde_json::from_str(&crate::cli::metadata::report_json(&project, true).expect("metadata"))
            .expect("metadata is JSON");
    let packages = default_only["packages"]
        .as_array()
        .expect("packages is an array");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["name"], "app");
}

#[test]
fn metadata_is_deterministic() {
    let fixture = fixture("metadata-deterministic");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"demo\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/main.sl"),
        "func main() -> i32 { 0 }\n",
    );
    let project = load(&fixture);
    let first = crate::cli::metadata::report_json(&project, false).expect("metadata");
    let second = crate::cli::metadata::report_json(&project, false).expect("metadata");
    assert_eq!(first, second);
}
