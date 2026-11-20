//! Feature resolution across package instances.

use super::{fixture, load, options};

fn workspace(
    left: &str,
    right: &str,
    app_dependencies: &str,
    common_features: &str,
) -> Vec<(String, String)> {
    vec![
        (
            "seal.toml".to_owned(),
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n"
                .to_owned(),
        ),
        (
            "crates/common/seal.toml".to_owned(),
            format!(
                "[package]\nname = \"common\"\nversion = \"0.1.0\"\n[package.features]\n{common_features}"
            ),
        ),
        (
            "crates/common/src/lib.sl".to_owned(),
            "pub fn value() -> Int { 1 }\n".to_owned(),
        ),
        (
            "crates/left/seal.toml".to_owned(),
            format!(
                "[package]\nname = \"left\"\nversion = \"0.1.0\"\n[dependencies]\ncommon = {{ path = \"../common\", features = [{left}] }}\n"
            ),
        ),
        ("crates/left/src/lib.sl".to_owned(), "pub fn left() -> Int { 1 }\n".to_owned()),
        (
            "crates/right/seal.toml".to_owned(),
            format!(
                "[package]\nname = \"right\"\nversion = \"0.1.0\"\n[dependencies]\ncommon = {{ path = \"../common\", features = [{right}] }}\n"
            ),
        ),
        ("crates/right/src/lib.sl".to_owned(), "pub fn right() -> Int { 1 }\n".to_owned()),
        (
            "crates/app/seal.toml".to_owned(),
            format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\n{app_dependencies}"
            ),
        ),
        (
            "crates/app/src/main.sl".to_owned(),
            "fn main() -> Int { 0 }\n".to_owned(),
        ),
    ]
}

fn files(left: &str, right: &str, app: &str, common: &str) -> Vec<(&'static str, &'static str)> {
    workspace(left, right, app, common)
        .into_iter()
        .map(|(path, contents)| {
            (
                Box::leak(path.into_boxed_str()) as &'static str,
                Box::leak(contents.into_boxed_str()) as &'static str,
            )
        })
        .collect()
}

#[test]
fn dependencies_with_different_features_get_distinct_instances() {
    let files = files(
        "\"a\"",
        "\"b\"",
        "left = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        "default = []\na = []\nb = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect("the plan resolves");
    let common: Vec<_> = plan
        .packages
        .iter()
        .filter(|package| package.name == "common")
        .collect();
    assert_eq!(common.len(), 2);
    assert!(
        common
            .iter()
            .any(|package| package.cfg.features.iter().any(|feature| feature == "a"))
    );
    assert!(
        common
            .iter()
            .any(|package| package.cfg.features.iter().any(|feature| feature == "b"))
    );
    assert_ne!(common[0].instance, common[1].instance);
    assert_eq!(common[0].files[0].path, common[1].files[0].path);
}

#[test]
fn equal_feature_sets_share_one_instance() {
    let files = files(
        "\"a\"",
        "\"a\"",
        "left = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        "default = []\na = []\nb = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect("the plan resolves");
    assert_eq!(
        plan.packages
            .iter()
            .filter(|package| package.name == "common")
            .count(),
        1
    );
}

#[test]
fn a_default_feature_is_not_enabled_when_disabled() {
    let files = files(
        "\"\"",
        "\"\"",
        "common = { path = \"../common\", default-features = false }\n",
        "default = [\"a\"]\na = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect("the plan resolves");
    let common = plan
        .packages
        .iter()
        .find(|package| package.name == "common")
        .expect("common is present");
    assert!(common.cfg.features.is_empty());
}

#[test]
fn a_dependency_feature_enables_its_dependency() {
    let files = files(
        "\"\"",
        "\"\"",
        "common = { path = \"../common\", optional = true }\n[package.features]\ndefault = []\nuse_common = [\"dep:common\"]\n",
        "default = []\na = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let without = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect("the default plan resolves");
    assert!(
        !without
            .packages
            .iter()
            .any(|package| package.name == "common")
    );
    let with = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&["use_common"]),
        )
        .expect("the feature plan resolves");
    assert!(with.packages.iter().any(|package| package.name == "common"));
}

#[test]
fn a_slash_feature_activates_the_dependency_feature() {
    let files = files(
        "\"\"",
        "\"\"",
        "common = { path = \"../common\" }\n[package.features]\ndefault = []\nuse_a = [\"common/a\"]\n",
        "default = []\na = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&["use_a"]),
        )
        .expect("the dependency feature resolves");
    let common = plan
        .packages
        .iter()
        .find(|package| package.name == "common")
        .expect("common is present");
    assert!(common.cfg.features.iter().any(|feature| feature == "a"));
}

#[test]
fn a_weak_feature_does_not_activate_an_optional_dependency() {
    let files = files(
        "\"\"",
        "\"\"",
        "common = { path = \"../common\", optional = true }\n[package.features]\ndefault = []\nuse_weak = [\"common?/a\"]\n",
        "default = []\na = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&["use_weak"]),
        )
        .expect("the weak feature resolves");
    assert!(!plan.packages.iter().any(|package| package.name == "common"));
}

#[test]
fn conflicting_features_are_reported() {
    let files = files(
        "\"a\", \"b\"",
        "\"\"",
        "common = { path = \"../common\", features = [\"a\", \"b\"] }\n",
        "default = []\na = []\nb = []\nconflicts = { a = [\"b\"] }\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let error = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect_err("the features conflict");
    assert_eq!(error.code(), "E8007");
    assert!(error.message().contains("conflict"));
}

#[test]
fn the_feature_variant_cap_reports_the_limit() {
    let common_features = (1..=9)
        .map(|index| format!("f{index} = []"))
        .collect::<Vec<_>>()
        .join("\n");
    let dependencies = (1..=9)
        .map(|index| format!("c{index} = {{ path = \"../common\", features = [\"f{index}\"] }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let app = format!("[dependencies]\n{dependencies}\n");
    let root = (
        "seal.toml",
        "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n",
    );
    let common_manifest = (
        "crates/common/seal.toml",
        &*Box::leak(
            format!("[package]\nname = \"common\"\nversion = \"0.1.0\"\n[package.features]\n{common_features}\n")
                .into_boxed_str(),
        ) as &'static str,
    );
    let app_manifest = (
        "crates/app/seal.toml",
        &*Box::leak(
            format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n{app}").into_boxed_str(),
        ) as &'static str,
    );
    let fixture = fixture(&[
        root,
        common_manifest,
        (
            "crates/common/src/lib.sl",
            "public func value() -> i32 { 1 }\n",
        ),
        app_manifest,
        ("crates/app/src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let project = load(&fixture);
    let error = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&[]),
        )
        .expect_err("the cap is exceeded");
    assert_eq!(error.code(), "E8007");
    assert!(error.message().contains("feature variants"));
}

#[test]
fn an_unknown_feature_is_reported() {
    let files = files(
        "\"\"",
        "\"\"",
        "common = { path = \"../common\" }\n",
        "default = []\n",
    );
    let fixture = fixture(&files);
    let project = load(&fixture);
    let error = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "app",
            &options(&["missing"]),
        )
        .expect_err("the feature is unknown");
    assert_eq!(error.code(), "E8007");
}
