//! Lockfile instance and checksum behavior.

use super::{fixture, load, options};

#[test]
fn the_lockfile_records_instances_features_and_edges() {
    let files = [
        (
            "seal.toml",
            "[project]\nname = \"lock\"\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n",
        ),
        (
            "crates/common/seal.toml",
            "[package]\nname = \"common\"\nversion = \"0.1.0\"\n\n[package.features]\ndefault = []\na = []\nb = []\n",
        ),
        (
            "crates/common/src/lib.sl",
            "public func value() -> i32 { 1 }\n",
        ),
        (
            "crates/left/seal.toml",
            "[package]\nname = \"left\"\nversion = \"0.1.0\"\n\n[dependencies]\ncommon = { path = \"../common\", features = [\"a\"] }\n",
        ),
        (
            "crates/left/src/lib.sl",
            "public func left() -> i32 { 1 }\n",
        ),
        (
            "crates/right/seal.toml",
            "[package]\nname = \"right\"\nversion = \"0.1.0\"\n\n[dependencies]\ncommon = { path = \"../common\", features = [\"b\"] }\n",
        ),
        (
            "crates/right/src/lib.sl",
            "public func right() -> i32 { 1 }\n",
        ),
        (
            "crates/app/seal.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nleft = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        ),
        ("crates/app/src/main.sl", "func main() -> i32 { 0 }\n"),
    ];
    let fixture = fixture(&files);
    let project = load(&fixture);
    let target = seal_target::host_triple();
    let lock = project.resolve_lockfile_with(target.as_str(), &options(&[]));
    let common: Vec<_> = lock
        .instances
        .iter()
        .filter(|instance| instance.name == "common")
        .collect();
    assert_eq!(common.len(), 2);
    assert!(
        common
            .iter()
            .any(|instance| instance.features.iter().any(|f| f == "a"))
    );
    assert!(
        common
            .iter()
            .any(|instance| instance.features.iter().any(|f| f == "b"))
    );
    let package = lock.package("common").expect("common is locked");
    assert!(package.features.iter().any(|feature| feature == "a"));
    assert!(package.features.iter().any(|feature| feature == "b"));
    assert!(!package.checksum.is_empty());

    let rendered = lock.render().expect("the lockfile renders");
    let reparsed = crate::Lockfile::parse(&rendered).expect("the lockfile parses");
    assert_eq!(reparsed, lock);
    assert_eq!(
        rendered,
        project
            .resolve_lockfile_with(target.as_str(), &options(&[]))
            .render()
            .expect("the lockfile renders twice")
    );
}
