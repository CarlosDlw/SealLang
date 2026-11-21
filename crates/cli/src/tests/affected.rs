//! Affected-path mapping tests.

use std::path::PathBuf;

use super::{load, monorepo, sample_project};

#[test]
fn a_source_change_affects_the_package_and_its_dependents() {
    let fixture = monorepo("affected-source");
    let project = load(&fixture);
    let changed = vec![PathBuf::from("crates/alpha/src/lib.sl")];
    let report = seal_project::affected_from_paths(&project, "HEAD", &changed);
    assert_eq!(report.packages, ["alpha", "beta", "app"]);
    assert_eq!(report.targets.len(), 3);
    assert!(
        report
            .targets
            .iter()
            .any(|target| target.package_name == "app")
    );
}

#[test]
fn a_manifest_or_lockfile_change_affects_every_package() {
    let fixture = monorepo("affected-global");
    let project = load(&fixture);
    for path in ["seal.lock", "seal.toml", "seal-toolchain.toml"] {
        let report = seal_project::affected_from_paths(&project, "HEAD", &[PathBuf::from(path)]);
        assert_eq!(report.packages, ["alpha", "beta", "app"]);
        assert_eq!(report.targets.len(), 3);
    }
}

#[test]
fn a_source_change_includes_every_target_kind_including_generators() {
    let fixture = sample_project("affected-generator");
    let project = load(&fixture);
    let report =
        seal_project::affected_from_paths(&project, "HEAD", &[PathBuf::from("src/lib.sl")]);
    let targets: Vec<&str> = report
        .targets
        .iter()
        .map(|target| target.target.as_str())
        .collect();
    assert!(targets.contains(&"codegen"), "targets: {targets:?}");
    assert!(targets.contains(&"checks"));
    assert!(targets.contains(&"measure"));
    assert!(targets.contains(&"demo"));
}

#[test]
fn strings_are_relative_and_sorted() {
    let fixture = monorepo("affected-paths");
    let project = load(&fixture);
    let report = seal_project::affected_from_paths(
        &project,
        "abc",
        &[
            PathBuf::from("/elsewhere/seal.toml"),
            PathBuf::from("crates/alpha/src/lib.sl"),
        ],
    );
    assert_eq!(report.since, "abc");
    let mut sorted = report.changed.clone();
    sorted.sort();
    assert_eq!(report.changed, sorted);
}
