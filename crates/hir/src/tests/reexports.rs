//! Re-exports: visibility, alias identity, cycles, and API exposure.

use seal_def::DefKind;

use crate::tests::support::{Fixture, child_module, function, item_in_module, root_module, tail};
use crate::{ExprKind, Hir, HirExprId, Res};

fn callee_res(hir: &Hir, expression: HirExprId) -> Res {
    match hir.expr(expression).kind.clone() {
        ExprKind::Call { callee, .. } => match hir.expr(callee).kind.clone() {
            ExprKind::Path { res, .. } => res,
            other => panic!("expected a path callee, got {other:?}"),
        },
        other => panic!("expected a call, got {other:?}"),
    }
}

#[test]
fn a_public_reexport_is_an_alias_for_its_canonical_item() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use api::answer;\nfunc main() -> i32 { answer() }",
        ),
        (
            "api.sl",
            "module api;\npublic use root::implementation::answer;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() -> i32 { 7 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.assert_clean();

    let root = root_module(&hir);
    let implementation = child_module(&hir, &fixture, root, "implementation");
    let answer = item_in_module(&hir, &fixture, implementation, DefKind::Function, "answer");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(answer));

    let alias = hir
        .api()
        .aliases()
        .iter()
        .find(|alias| fixture.symbol(alias.name) == "answer")
        .expect("the public re-export is recorded in the API");
    assert_eq!(alias.def, answer);
    assert_eq!(alias.path.display(&fixture.interner), "api");
}

#[test]
fn a_module_reexport_can_be_the_root_of_a_qualified_path() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() -> i32 { api::impl_api::answer() }"),
        (
            "api.sl",
            "module api;\npublic use root::implementation as impl_api;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() -> i32 { 9 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.assert_clean();

    let root = root_module(&hir);
    let api = child_module(&hir, &fixture, root, "api");
    let implementation = child_module(&hir, &fixture, root, "implementation");
    let answer = item_in_module(&hir, &fixture, implementation, DefKind::Function, "answer");
    assert_eq!(
        hir.child_module(api, fixture.interner.intern("impl_api")),
        Some(implementation)
    );
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(answer));
    assert!(hir.api().module_aliases().iter().any(|alias| {
        alias.module == implementation && alias.path.display(&fixture.interner) == "api.impl_api"
    }));
}

#[test]
fn a_type_reexport_keeps_the_canonical_type_identity() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use api::Point;\nfunc make() -> Point { Point { x: 1 } }",
        ),
        ("api.sl", "module api;\npublic use root::geometry::Point;"),
        (
            "geometry.sl",
            "module geometry;\npublic struct Point { public x: i32 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "geometry"]);
    fixture.assert_clean();

    let root = root_module(&hir);
    let geometry = child_module(&hir, &fixture, root, "geometry");
    let canonical = item_in_module(&hir, &fixture, geometry, DefKind::Struct, "Point");
    let (_, body) = function(&fixture, &hir, "make");
    let ret = body.ret.expect("the function has a return type");
    let adt = match hir.ty_ctxt().kind(ret) {
        seal_ty::TyKind::Adt { adt, .. } => adt,
        other => panic!("the return type is not an aggregate: {other:?}"),
    };
    assert_eq!(hir.ty_ctxt().adt(adt).def, canonical);
}

#[test]
fn an_internal_reexport_is_visible_inside_its_package_without_exposing_modules() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use root::api::value;\nfunc main() -> i32 { value() }",
        ),
        (
            "api.sl",
            "module api;\ninternal use root::implementation::value;",
        ),
        (
            "implementation.sl",
            "module implementation;\ninternal func value() -> i32 { 5 }",
        ),
    ]);
    let hir = fixture.lower_package();
    fixture.assert_clean();
    let root = root_module(&hir);
    let implementation = child_module(&hir, &fixture, root, "implementation");
    let value = item_in_module(&hir, &fixture, implementation, DefKind::Function, "value");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(value));
    assert!(hir.api().aliases().is_empty());
}

#[test]
fn a_public_reexport_of_a_non_public_item_is_rejected() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::implementation::hidden;",
        ),
        (
            "implementation.sl",
            "module implementation;\ninternal func hidden() {}",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.expect("E2092", "targets an item that is not public");
}

#[test]
fn a_public_reexport_requires_an_exposed_declaring_module() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::implementation::answer;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() {}",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["implementation"]);
    fixture.expect("E2088", "its module is not exposed");
}

#[test]
fn a_public_reexport_requires_an_exposed_target_module() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::implementation::answer;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() {}",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api"]);
    fixture.expect("E2093", "reaches a module that is not exposed");
}

#[test]
fn a_reexport_chain_is_resolved_to_its_canonical_target() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() -> i32 { root::api::value() }"),
        ("api.sl", "module api;\npublic use root::facade::value;"),
        (
            "facade.sl",
            "module facade;\npublic use root::implementation::value;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func value() -> i32 { 4 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "facade", "implementation"]);
    fixture.assert_clean();
    let root = root_module(&hir);
    let implementation = child_module(&hir, &fixture, root, "implementation");
    let value = item_in_module(&hir, &fixture, implementation, DefKind::Function, "value");
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(callee_res(&hir, tail(&hir, body)), Res::Function(value));
}

#[test]
fn a_reexport_cycle_is_rejected() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        ("first.sl", "module first;\npublic use root::second::value;"),
        (
            "second.sl",
            "module second;\npublic use root::first::value;",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["first", "second"]);
    fixture.expect("E2089", "part of a cycle");
}

#[test]
fn duplicate_reexports_are_rejected() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::implementation::value;\npublic use root::implementation::value as value;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func value() {}",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.expect("E2090", "declared twice");
}

#[test]
fn a_diamond_reexport_keeps_both_aliases() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::left::value;\npublic use root::right::value as other;",
        ),
        (
            "left.sl",
            "module left;\npublic use root::implementation::value;",
        ),
        (
            "right.sl",
            "module right;\npublic use root::implementation::value;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func value() {}",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "left", "right", "implementation"]);
    fixture.assert_clean();
    let root = root_module(&hir);
    let implementation = child_module(&hir, &fixture, root, "implementation");
    let canonical = item_in_module(&hir, &fixture, implementation, DefKind::Function, "value");
    let mut aliases: Vec<_> = hir
        .api()
        .aliases()
        .iter()
        .filter(|alias| alias.def == canonical && alias.path.display(&fixture.interner) == "api")
        .map(|alias| fixture.symbol(alias.name))
        .collect();
    aliases.sort();
    assert_eq!(aliases, ["other", "value"]);
}

#[test]
fn a_re_exported_module_can_be_exposed_by_its_alias_path() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "facade.sl",
            "module facade;\npublic use root::implementation as impl_api;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() -> i32 { 1 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["facade::impl_api", "implementation"]);
    fixture.assert_clean();
    let root = root_module(&hir);
    let facade = child_module(&hir, &fixture, root, "facade");
    let implementation = child_module(&hir, &fixture, root, "implementation");
    assert_eq!(
        hir.child_module(facade, fixture.interner.intern("impl_api")),
        Some(implementation)
    );
    assert!(hir.api().module_aliases().iter().any(|alias| {
        alias.module == implementation && alias.path.display(&fixture.interner) == "facade.impl_api"
    }));
}

#[test]
fn an_alias_of_a_root_reexport_is_listed_in_the_api() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "public use root::implementation as impl_api;\nfunc main() -> i32 { root::impl_api::answer() }",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() -> i32 { 3 }",
        ),
    ]);
    let hir = fixture.lower_package_exposing(&["root", "implementation"]);
    fixture.assert_clean();
    let root = root_module(&hir);
    let implementation = child_module(&hir, &fixture, root, "implementation");
    assert!(hir.api().module_aliases().iter().any(|alias| {
        alias.module == implementation && alias.path.display(&fixture.interner) == "impl_api"
    }));
}

#[test]
fn a_variant_cannot_be_re_exported_as_a_standalone_name() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() {}"),
        (
            "api.sl",
            "module api;\npublic use root::implementation::Choice::One;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic enum Choice { One } ",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.expect("E2091", "is not a module");
}

#[test]
fn a_cfg_disabled_reexport_is_not_part_of_the_graph() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use api::answer;\nfunc main() -> i32 { answer() }",
        ),
        (
            "api.sl",
            "module api;\ncfg(feature(\"extra\")) public use root::implementation::answer;",
        ),
        (
            "implementation.sl",
            "module implementation;\npublic func answer() -> i32 { 7 }",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api", "implementation"]);
    fixture.expect("E2002", "cannot find `answer`");
}
