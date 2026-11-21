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
    // Each library contributes its library and automatic unit-test target;
    // the application contributes its binary target.
    assert_eq!(report.targets.len(), 5);
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
        assert_eq!(report.targets.len(), 5);
    }
}

#[test]
fn a_library_change_maps_library_and_generator_targets() {
    let fixture = sample_project("affected-generator");
    let project = load(&fixture);
    let report =
        seal_project::affected_from_paths(&project, "HEAD", &[PathBuf::from("src/lib.sl")]);
    let targets: Vec<&str> = report
        .targets
        .iter()
        .map(|target| target.target.as_str())
        .collect();
    assert!(targets.contains(&"share"), "targets: {targets:?}");
    assert!(targets.contains(&"codegen"), "targets: {targets:?}");
    // Integration tests own only their file; changing the library does not
    // make unrelated test sources dirty.
    assert!(!targets.contains(&"checks"), "targets: {targets:?}");
    assert!(!targets.contains(&"measure"), "targets: {targets:?}");
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
