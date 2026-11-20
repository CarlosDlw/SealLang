//! `cfg(...)` filtering and environment validation.

use std::collections::BTreeMap;

use seal_def::DefKind;

use crate::CfgEnvironment;
use crate::tests::support::{Fixture, item_in_module, root_module};

fn environment(features: &[&str]) -> CfgEnvironment {
    CfgEnvironment {
        features: features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect(),
        target_os: "linux".to_owned(),
        target_arch: "x86_64".to_owned(),
        pointer_width: 64,
        debug_assertions: true,
        env: BTreeMap::new(),
        ..CfgEnvironment::default()
    }
}

#[test]
fn an_inactive_function_is_not_lowered() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(feature(\"extra\")) func hidden() -> i32 { missing() }\nfunc main() -> i32 { 0 }",
    )]);
    let hir = fixture.lower_package_with_cfg(environment(&[]));
    fixture.assert_clean();
    let root = root_module(&hir);
    assert!(
        hir.definitions()
            .iter()
            .all(|definition| definition.kind != DefKind::Function
                || fixture.symbol(definition.name) != "hidden")
    );
    let _ = item_in_module(&hir, &fixture, root, DefKind::Function, "main");
}

#[test]
fn an_active_function_is_lowered() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(feature(\"extra\")) func hidden() -> i32 { 1 }\nfunc main() -> i32 { 0 }",
    )]);
    let hir = fixture.lower_package_with_cfg(environment(&["extra"]));
    fixture.assert_clean();
    let root = root_module(&hir);
    let definition = item_in_module(&hir, &fixture, root, DefKind::Function, "hidden");
    assert!(hir.definitions().get(definition).cfg.is_some());
}

#[test]
fn an_inactive_module_is_not_created() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() -> i32 { 0 }"),
        (
            "hidden.sl",
            "cfg(feature(\"extra\")) module app::hidden;\npublic func hidden() -> i32 { 1 }",
        ),
    ]);
    let hir = fixture.lower_package_with_cfg(environment(&[]));
    fixture.assert_clean();
    assert_eq!(hir.modules().len(), 1);
}

#[test]
fn an_inactive_import_is_ignored() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(feature(\"extra\")) use absent::Thing;\nfunc main() -> i32 { 0 }",
    )]);
    let hir = fixture.lower_package_with_cfg(environment(&[]));
    fixture.assert_clean();
    let root = root_module(&hir);
    item_in_module(&hir, &fixture, root, DefKind::Function, "main");
}

#[test]
fn an_undeclared_environment_value_is_reported() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(env(\"CHANNEL\") == \"nightly\") func nightly() {}\nfunc main() -> i32 { 0 }",
    )]);
    let _ = fixture.lower_package_with_cfg(environment(&[]));
    assert_eq!(fixture.codes(), ["E2084"]);
}

#[test]
fn a_declared_environment_value_can_be_compared() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(env(\"CHANNEL\") == \"nightly\") func nightly() {}\nfunc main() -> i32 { 0 }",
    )]);
    let mut cfg = environment(&[]);
    cfg.env.insert("CHANNEL".to_owned(), "nightly".to_owned());
    let hir = fixture.lower_package_with_cfg(cfg);
    fixture.assert_clean();
    let root = root_module(&hir);
    item_in_module(&hir, &fixture, root, DefKind::Function, "nightly");
}

#[test]
fn target_and_boolean_conditions_are_evaluated() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "cfg(target.os == \"linux\" && debug_assertions) func linux() {}\nfunc main() -> i32 { 0 }",
    )]);
    let hir = fixture.lower_package_with_cfg(environment(&[]));
    fixture.assert_clean();
    let root = root_module(&hir);
    item_in_module(&hir, &fixture, root, DefKind::Function, "linux");
}
