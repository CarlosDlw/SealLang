//! Imports: absolute roots, aliases, file scope, collisions, and dependencies.

use seal_def::{DefKind, PackageId};
use seal_index::Idx;

use crate::tests::support::{
    Fixture, PackageSpec, child_module, function, item_in_module, lower_package_clean, path_res,
    root_module, tail,
};
use crate::{ExprKind, Hir, HirExprId, Res};

/// Returns the resolution of a call's callee.
fn callee_res(hir: &Hir, expression: HirExprId) -> Res {
    match hir.expr(expression).kind.clone() {
        ExprKind::Call { callee, .. } => path_res(hir, callee),
        other => panic!("expected a call, got {other:?}"),
    }
}

#[test]
fn an_import_can_start_at_the_package_root() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "use root::helper as renamed;\nfunc main() -> i32 { renamed() }",
        ),
        ("helper.sl", "internal func helper() -> i32 { 7 }"),
    ]);

    let root = root_module(&hir);
    let helper = item_in_module(&hir, &fixture, root, DefKind::Function, "helper");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(helper));
}

#[test]
fn a_qualified_expression_reaches_the_root() {
    let (fixture, hir) = lower_package_clean(&[
        ("main.sl", "func main() -> i32 { root::helper() }"),
        ("helper.sl", "internal func helper() -> i32 { 7 }"),
    ]);

    let root = root_module(&hir);
    let helper = item_in_module(&hir, &fixture, root, DefKind::Function, "helper");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(helper));
}

#[test]
fn a_qualified_type_can_name_a_nested_module() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "func make() -> app::util::Point { app::util::Point { x: 1 } }",
        ),
        (
            "util.sl",
            "module app::util;\ninternal struct Point { internal x: i32 }",
        ),
    ]);

    let root = root_module(&hir);
    let app = child_module(&hir, &fixture, root, "app");
    let util = child_module(&hir, &fixture, app, "util");
    let point = item_in_module(&hir, &fixture, util, DefKind::Struct, "Point");
    let (_, body) = function(&fixture, &hir, "make");
    let ret = body.ret.expect("the function declares a return type");
    let adt = match hir.ty_ctxt().kind(ret) {
        seal_ty::TyKind::Adt { adt, .. } => adt,
        other => panic!("the return type is not an aggregate: {other:?}"),
    };
    assert_eq!(hir.ty_ctxt().adt(adt).def, point);
}

#[test]
fn a_group_alias_renames_only_its_own_binding() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "use app::{value as answer};\nfunc main() -> i32 { answer() }",
        ),
        ("app.sl", "module app;\ninternal func value() -> i32 { 1 }"),
    ]);

    let root = root_module(&hir);
    let app = child_module(&hir, &fixture, root, "app");
    let value = item_in_module(&hir, &fixture, app, DefKind::Function, "value");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(value));
}

#[test]
fn imports_are_not_shared_between_fragments_of_one_module() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "first.sl",
            "module app;\nuse util::value;\ninternal func first() -> i32 { value() }",
        ),
        (
            "second.sl",
            "module app;\ninternal func second() -> i32 { value() }",
        ),
        (
            "util.sl",
            "module util;\ninternal func value() -> i32 { 3 }",
        ),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2002", "cannot find `value`");
}

#[test]
fn an_import_that_resolution_never_uses_is_a_warning() {
    let fixture = Fixture::package(&[
        ("main.sl", "use app::unused;\nfunc main() {}"),
        ("app.sl", "module app;\ninternal func unused() -> i32 { 1 }"),
    ]);
    let _ = fixture.lower_package();
    let warning = fixture
        .diagnostics()
        .into_iter()
        .find(|diagnostic| diagnostic.message().contains("never used"))
        .expect("the unused import is reported");
    assert!(warning.code().is_none());
    assert_eq!(warning.level(), seal_diagnostics::Level::Warning);
    assert_eq!(warning.lint(), Some(seal_diagnostics::UNUSED_IMPORT));
}

#[test]
fn an_import_name_cannot_replace_an_item_of_the_module() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use other::thing;\nfunc thing() {}\nfunc main() {}",
        ),
        ("other.sl", "module other;\ninternal func thing() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2077", "conflicts with an item of this module");
}

#[test]
fn one_file_cannot_bind_the_same_import_name_twice() {
    let fixture = Fixture::package(&[
        ("main.sl", "use other::{thing, thing};\nfunc main() {}"),
        ("other.sl", "module other;\ninternal func thing() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2078", "already used by another import");
}

#[test]
fn a_name_that_is_both_a_module_and_an_item_is_ambiguous() {
    let fixture = Fixture::package(&[
        ("main.sl", "use app::util;\nfunc main() {}"),
        ("app.sl", "module app;\ninternal func util() {}"),
        ("util.sl", "module app::util;\ninternal func inside() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2074", "denotes both a module and an item");
}

#[test]
fn a_dependency_alias_reaches_its_exposed_module() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "geometry",
            files: &[("geometry.sl", "public func distance() -> i32 { 7 }")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::distance;\nfunc main() -> i32 { distance() }",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let hir = fixture.lower_packages();
    fixture.assert_clean();

    let dependency = hir.package(PackageId::from_usize(0)).root;
    let distance = item_in_module(&hir, &fixture, dependency, DefKind::Function, "distance");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(distance));
}

#[test]
fn a_private_dependency_module_cannot_be_imported() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "geometry",
            files: &[(
                "geometry.sl",
                "module geometry;\npublic func distance() -> i32 { 7 }",
            )],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::geometry::distance;\nfunc main() {}",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.expect("E2080", "is not exposed");
}

#[test]
fn a_dependency_alias_cannot_shadow_a_top_level_module() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "dep",
            files: &[("dep.sl", "module dep;\ninternal func helper() {}")],
            exposed: &[],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[("main.sl", "module geometry;\nfunc main() {}")],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.expect("E2077", "conflicts with a top-level module");
}

#[test]
fn modules_may_import_each_other_in_a_cycle() {
    let (fixture, _) = lower_package_clean(&[
        ("main.sl", "use a::one;\nfunc main() -> i32 { one() }"),
        (
            "a.sl",
            "module a;\nuse b::two;\ninternal func one() -> i32 { two() }",
        ),
        (
            "b.sl",
            "module b;\nuse a::one;\ninternal func two() -> i32 { one() }",
        ),
    ]);
    let _ = fixture;
}

#[test]
fn a_dependency_item_that_is_not_public_is_rejected() {
    let fixture = Fixture::from_packages(&[
        PackageSpec {
            name: "geometry",
            files: &[(
                "geometry.sl",
                "module geometry;\ninternal func distance() -> i32 { 7 }",
            )],
            exposed: &["geometry"],
            dependencies: &[],
        },
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::geometry::distance;\nfunc main() {}",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.expect("E2072", "not visible");
}
