//! Unit tests for the project CLI.
//!
//! These tests exercise the command internals directly. The end-to-end tests
//! in `crates/cli/tests` cover the process boundary and argument parsing.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use seal_project::Project;

use crate::cli::Selection;

mod affected;
mod build;
mod docs;
mod graph;
mod metadata;
mod new;
mod run;
mod semver;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary project root removed at the end of a test.
pub(super) struct Fixture {
    pub(super) root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Creates an empty temporary directory.
pub(super) fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-cli-unit-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temporary root");
    Fixture { root }
}

/// Writes one fixture file, creating its parent directories.
pub(super) fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture parent");
    }
    std::fs::write(path, contents).expect("fixture file");
}

/// Writes a single-package project with the usual target layout.
pub(super) fn sample_project(name: &str) -> Fixture {
    let fixture = fixture(name);
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"demo\"\n\n\
         [package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
         [targets.share]\nkind = \"lib\"\n\n\
         [targets.main]\nkind = \"bin\"\nentry = \"root::main\"\nprofile = \"release\"\n\n\
         [targets.checks]\nkind = \"test\"\n\n\
         [targets.measure]\nkind = \"bench\"\n\n\
         [targets.demo]\nkind = \"example\"\nentry = \"root::main\"\n\n\
         [targets.codegen]\nkind = \"generator\"\nentry = \"root::main\"\noutputs = [\"generated/codegen.sl\"]\n\n\
         [profiles.release]\nopt-level = 3\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func answer() -> i32 { 42 }\n",
    );
    write(
        &fixture.root.join("src/main.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("tests/checks.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("benches/measure.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("examples/demo.sl"),
        "func main() -> i32 { 0 }\n",
    );
    fixture
}

/// Writes a three-package workspace with a linear dependency chain.
pub(super) fn monorepo(name: &str) -> Fixture {
    let fixture = fixture(name);
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"mono\"\nmembers = [\"crates/*\"]\n\
         default-members = [\"crates/alpha\", \"crates/beta\"]\n",
    );
    write(
        &fixture.root.join("crates/alpha/seal.toml"),
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("crates/alpha/src/lib.sl"),
        "//! Alpha helpers.\n\n/// Returns one.\npublic func one() -> i32 { 1 }\n",
    );
    write(
        &fixture.root.join("crates/beta/seal.toml"),
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\n\n\
         [dependencies]\nalpha = { path = \"../alpha\" }\n",
    );
    write(
        &fixture.root.join("crates/beta/src/lib.sl"),
        "use alpha::one;\n\n/// Returns two.\npublic func two() -> i32 { one() + 1 }\n",
    );
    write(
        &fixture.root.join("crates/app/seal.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies]\nbeta = { path = \"../beta\" }\n",
    );
    write(
        &fixture.root.join("crates/app/src/main.sl"),
        "use beta::two;\n\nfunc main() -> i32 { two() }\n",
    );
    fixture
}

/// Loads a fixture as a project.
pub(super) fn load(fixture: &Fixture) -> Project {
    Project::load(&fixture.root).expect("the fixture is a valid project")
}

#[test]
fn no_prelude_is_a_build_selection_flag() {
    let cli = crate::cli::Cli::parse_from(["seal", "build", "--no-prelude", "main.sl"]);
    match cli.command {
        crate::cli::Command::Build(build) => assert!(build.selection.no_prelude),
        other => panic!("expected a build command, got {other:?}"),
    }
}

/// Returns a selection with no filters, as if the user passed no flags.
pub(super) fn selection() -> Selection {
    Selection {
        targets: Vec::new(),
        package: None,
        workspace: false,
        exclude: Vec::new(),
        all_targets: false,
        profile: None,
        release: false,
        target_triple: None,
        features: Vec::new(),
        all_features: false,
        no_default_features: false,
        no_prelude: false,
        cfgs: Vec::new(),
        locked: false,
        frozen: false,
    }
}
