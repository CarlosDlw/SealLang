//! Scaffolding tests for `seal new` and `seal init`.

use seal_manifest::TargetKind;

use crate::cli::Template;

use super::{fixture, load};

#[test]
fn every_template_writes_the_current_edition() {
    for (name, template) in [
        ("new-edition-bin", Template::Bin),
        ("new-edition-lib", Template::Lib),
        ("new-edition-workspace", Template::Workspace),
        ("new-edition-kernel", Template::Kernel),
        ("new-edition-compiler", Template::Compiler),
    ] {
        let fixture = fixture(name);
        crate::cli::new::scaffold(&fixture.root, name, template)
            .unwrap_or_else(|error| panic!("{name} scaffold failed: {error}"));
        let project = load(&fixture);
        assert!(!project.packages.is_empty(), "{name} has packages");
        assert!(
            project
                .packages
                .iter()
                .all(|package| package.edition.to_string() == "2026"),
            "{name} writes the current edition"
        );
    }
}

#[test]
fn a_binary_scaffold_is_a_loadable_multi_file_project() {
    let fixture = fixture("new-bin");
    crate::cli::new::scaffold(&fixture.root, "demo-app", Template::Bin)
        .expect("the binary scaffold is written");

    let project = load(&fixture);
    assert_eq!(project.name, "demo-app");
    let package = project.package(0);
    assert_eq!(package.name, "demo-app");
    assert_eq!(package.targets[0].kind, TargetKind::Bin);
    assert_eq!(package.targets[0].entry.as_deref(), Some("root::main"));
    assert!(fixture.root.join("src/main.sl").is_file());
    assert!(fixture.root.join("src/util.sl").is_file());
    assert!(fixture.root.join("tests").is_dir());
    assert!(fixture.root.join(".gitignore").is_file());
    assert!(fixture.root.join("seal.lock").is_file());
}

#[test]
fn a_library_scaffold_has_no_main_and_exposes_the_root() {
    let fixture = fixture("new-lib");
    crate::cli::new::scaffold(&fixture.root, "demo-lib", Template::Lib)
        .expect("the library scaffold is written");

    let project = load(&fixture);
    let package = project.package(0);
    assert_eq!(package.targets[0].kind, TargetKind::Lib);
    assert_eq!(package.targets[0].entry, None);
    assert!(!fixture.root.join("src/main.sl").exists());
    assert!(fixture.root.join("src/lib.sl").is_file());
    assert!(package.manifest.package.as_ref().is_some_and(|manifest| {
        manifest
            .api
            .as_ref()
            .is_some_and(|api| api.expose.iter().any(|module| module == "root"))
    }));
}

#[test]
fn a_workspace_scaffold_builds_a_path_dependency() {
    let fixture = fixture("new-workspace");
    crate::cli::new::scaffold(&fixture.root, "demo-workspace", Template::Workspace)
        .expect("the workspace scaffold is written");

    let project = load(&fixture);
    assert_eq!(project.packages.len(), 2);
    let app = project.package_by_name("app").expect("the app member");
    let greet = project.package_by_name("greet").expect("the greet member");
    assert_eq!(app.1.dependencies.len(), 1);
    assert_eq!(app.1.dependencies[0].alias, "greet");
    assert_eq!(greet.1.name, "greet");
    assert!(fixture.root.join("crates/app/src/main.sl").is_file());
    assert!(fixture.root.join("crates/greet/src/lib.sl").is_file());
    assert!(fixture.root.join("seal.lock").is_file());
}

#[test]
fn scaffolding_twice_replaces_the_same_generated_files() {
    let fixture = fixture("new-twice");
    crate::cli::new::scaffold(&fixture.root, "demo", Template::Bin)
        .expect("the first scaffold is written");
    crate::cli::new::scaffold(&fixture.root, "demo", Template::Bin)
        .expect("the second scaffold replaces the generated files");

    let project = load(&fixture);
    assert_eq!(project.package(0).targets[0].kind, TargetKind::Bin);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("src/main.sl"))
            .expect("main.sl is readable")
            .matches("func main")
            .count(),
        1
    );
}
