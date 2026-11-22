//! Prelude selection, precedence, and diagnostics.

use seal_def::PackageId;
use seal_index::Idx;

use crate::tests::support::{Fixture, PackageSpec, function, tail};
use crate::{ExprKind, Hir, HirExprId, ModulePath, PreludeSource, Res};

fn callee(hir: &Hir, expression: HirExprId) -> seal_def::DefId {
    match hir.expr(expression).kind.clone() {
        ExprKind::Call { callee, .. } => match hir.expr(callee).kind.clone() {
            ExprKind::Path {
                res: Res::Function(definition),
                ..
            } => definition,
            other => panic!("expected a function path, got {other:?}"),
        },
        other => panic!("expected a call, got {other:?}"),
    }
}

fn core_option() -> PackageSpec {
    PackageSpec {
        name: "core",
        files: &[(
            "core.sl",
            "public struct Option { public value: i32 }\n\
             public struct Result { public value: i32 }",
        )],
        exposed: &[],
        dependencies: &[],
    }
}

#[test]
fn a_synthetic_prelude_makes_option_and_result_available() {
    let fixture = Fixture::from_packages(&[
        core_option(),
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "func main() -> i32 {\n\
                 let option = Option { value: 1 };\n\
                 let result = Result { value: 2 };\n\
                 option.value + result.value\n\
                 }",
            )],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let hir = fixture.lower_packages_with_prelude(1, 0, "root");
    fixture.assert_clean();
    let table = hir
        .prelude(hir.primary_package())
        .expect("the primary package has a prelude");
    assert_eq!(table.len(), 2);
    assert_eq!(table.source().package, PackageId::from_usize(0));
}

#[test]
fn an_explicit_import_wins_over_the_prelude() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[("core.sl", "public func answer() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "other",
            files: &[("other.sl", "public func answer() -> i32 { 2 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use other::answer;\nfunc main() -> i32 { answer() }",
            )],
            exposed: &[],
            dependencies: &[("core", 0), ("other", 1)],
        },
    ]);
    let hir = fixture.lower_packages_with_prelude(2, 0, "root");
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let target = callee(&hir, tail(&hir, body));
    assert_eq!(hir.package_of_def(target), PackageId::from_usize(1));
}

#[test]
fn a_local_item_wins_over_the_prelude() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[("core.sl", "public func answer() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "internal func answer() -> i32 { 3 }\n\
                 func main() -> i32 { answer() }",
            )],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let hir = fixture.lower_packages_with_prelude(1, 0, "root");
    fixture.assert_clean();
    let (_, body) = function(&fixture, &hir, "main");
    let target = callee(&hir, tail(&hir, body));
    assert_eq!(hir.package_of_def(target), hir.primary_package());
}

#[test]
fn a_value_named_option_does_not_shadow_the_prelude_type() {
    let fixture = Fixture::from_packages(&[
        core_option(),
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "func Option() -> i32 { 1 }\n\
                 func main() -> i32 {\n\
                 let option = Option { value: 2 };\n\
                 option.value + Option()\n\
                 }",
            )],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let _hir = fixture.lower_packages_with_prelude(1, 0, "root");
    fixture.assert_clean();
}

#[test]
fn an_unused_prelude_name_does_not_emit_a_warning() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[("core.sl", "public func answer() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[("main.sl", "func main() {}")],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let _hir = fixture.lower_packages_with_prelude(1, 0, "root");
    fixture.assert_clean();
}

#[test]
fn an_import_that_only_shadows_the_prelude_is_still_unused() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[("core.sl", "public func answer() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "other",
            files: &[("other.sl", "public func answer() -> i32 { 2 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[("main.sl", "use other::answer;\nfunc main() {}")],
            exposed: &[],
            dependencies: &[("core", 0), ("other", 1)],
        },
    ]);
    let _hir = fixture.lower_packages_with_prelude(2, 0, "root");
    assert!(
        fixture
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message().contains("never used"))
    );
    assert!(
        fixture
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().is_none())
    );
}

#[test]
fn a_module_export_in_the_prelude_is_e2101() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[
                (
                    "prelude.sl",
                    "module prelude;\npublic use root::inner as Hidden;",
                ),
                ("inner.sl", "module inner;"),
            ],
            exposed: &["prelude", "inner"],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[("main.sl", "func main() {}")],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let _hir = fixture.lower_packages_with_prelude(1, 0, "prelude");
    fixture.expect("E2101", "is a module");
}

#[test]
fn a_missing_prelude_module_is_e2100() {
    let fixture = Fixture::from_packages(&[
        core_option(),
        PackageSpec {
            name: "app",
            files: &[("main.sl", "func main() {}")],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let _hir = fixture.lower_packages_with_prelude(1, 0, "missing");
    fixture.expect("E2100", "does not exist");
}

#[test]
fn a_package_without_a_prelude_compiles_as_before() {
    let fixture = Fixture::from_packages(&[
        core_option(),
        PackageSpec {
            name: "app",
            files: &[("main.sl", "func main() -> i32 { 0 }")],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let hir = fixture.lower_packages();
    fixture.assert_clean();
    assert!(hir.prelude(hir.primary_package()).is_none());
}

#[test]
fn test_and_bench_instances_receive_the_same_prelude() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "core",
            files: &[("core.sl", "public func answer() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[("main.sl", "func main() -> i32 { answer() }")],
            exposed: &[],
            dependencies: &[("core", 0)],
        },
    ]);
    let preludes = [(
        1,
        PreludeSource {
            package: PackageId::from_usize(0),
            module: ModulePath::root(),
        },
    )];
    let test = crate::CfgEnvironment {
        test: true,
        ..crate::CfgEnvironment::default()
    };
    let test_hir = fixture.lower_packages_with_preludes_and_cfgs(&preludes, &[(1, test)]);
    fixture.assert_clean();
    assert!(test_hir.prelude(PackageId::from_usize(1)).is_some());

    let bench = crate::CfgEnvironment {
        bench: true,
        ..crate::CfgEnvironment::default()
    };
    let bench_hir = fixture.lower_packages_with_preludes_and_cfgs(&preludes, &[(1, bench)]);
    fixture.assert_clean();
    assert!(bench_hir.prelude(PackageId::from_usize(1)).is_some());
}

#[test]
fn a_cycle_between_prelude_packages_is_e2103() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "alpha",
            files: &[("alpha.sl", "public func alpha() -> i32 { 1 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "beta",
            files: &[("beta.sl", "public func beta() -> i32 { 2 }")],
            exposed: &[],
            dependencies: &[],
        },
    ]);
    let hir = fixture.lower_packages_with_preludes(&[
        (
            0,
            PreludeSource {
                package: PackageId::from_usize(1),
                module: ModulePath::root(),
            },
        ),
        (
            1,
            PreludeSource {
                package: PackageId::from_usize(0),
                module: ModulePath::root(),
            },
        ),
    ]);
    fixture.expect("E2103", "cycle");
    assert!(hir.prelude(PackageId::from_usize(0)).is_none());
    assert!(hir.prelude(PackageId::from_usize(1)).is_none());
}
