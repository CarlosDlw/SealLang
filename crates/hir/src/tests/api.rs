//! The public API: exposure, membership, and signature reachability.

use seal_def::{DefId, DefKind};

use crate::Hir;
use crate::tests::support::{Fixture, child_module, item_in_module, root_module};

fn app_module(fixture: &Fixture, hir: &Hir) -> crate::ModuleId {
    child_module(hir, fixture, root_module(hir), "app")
}

#[test]
fn a_public_root_item_is_api_when_the_root_is_exposed() {
    let fixture = Fixture::package(&[("main.sl", "public func answer() -> i32 { 42 }")]);
    let hir = fixture.lower_package();
    fixture.assert_clean();

    let answer = item_in_module(
        &hir,
        &fixture,
        root_module(&hir),
        DefKind::Function,
        "answer",
    );
    assert!(hir.is_public_api(answer));
    assert!(hir.public_items().contains(&answer));
}

#[test]
fn the_root_can_be_exposed_explicitly() {
    let fixture = Fixture::package(&[("main.sl", "public func answer() -> i32 { 42 }")]);
    let hir = fixture.lower_package_exposing(&["root"]);
    fixture.assert_clean();

    let answer = item_in_module(
        &hir,
        &fixture,
        root_module(&hir),
        DefKind::Function,
        "answer",
    );
    assert!(hir.is_public_api(answer));
}

#[test]
fn exposing_another_module_does_not_expose_the_root() {
    let fixture = Fixture::package(&[
        ("main.sl", "public func answer() -> i32 { 42 }"),
        ("app.sl", "module app;\ninternal func helper() {}"),
    ]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E2080", "not reachable");
}

#[test]
fn an_api_path_that_names_no_module_is_rejected() {
    let fixture = Fixture::package(&[("main.sl", "func main() {}")]);
    let _ = fixture.lower_package_exposing(&["missing"]);
    fixture.expect("E2081", "does not exist");
}

#[test]
fn exposing_a_leaf_also_exposes_its_ancestors() {
    let fixture = Fixture::package(&[(
        "util.sl",
        "module app::util;\npublic func answer() -> i32 { 42 }",
    )]);
    let hir = fixture.lower_package_exposing(&["app::util"]);
    fixture.assert_clean();

    let app = app_module(&fixture, &hir);
    let util = child_module(&hir, &fixture, app, "util");
    assert!(hir.module(app).exposed);
    assert!(hir.module(util).exposed);
    let answer = item_in_module(&hir, &fixture, util, DefKind::Function, "answer");
    assert!(hir.is_public_api(answer));
}

#[test]
fn a_public_return_type_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public func leak() -> Secret { Secret { value: 1 } }",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3100", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_value_type_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         internal const HIDDEN: Secret = Secret { value: 1 };\n\
         public const LEAK: Secret = HIDDEN;",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3101", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_trait_method_type_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public trait Listener { func take(value: Secret); }",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3102", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_trait_supertrait_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal trait Hidden {}\n\
         public trait Visible: Hidden {}",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3103", "exposes the non-public trait `Hidden`");
}

#[test]
fn a_public_field_type_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public struct Holder { public secret: Secret }",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3104", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_variant_type_must_be_public() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public enum Choice { Value(Secret) }",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3105", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_struct_keeps_private_fields_out_of_the_api() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public struct Holder { public shown: i32, private hidden: Secret }",
    )]);
    let hir = fixture.lower_package_exposing(&["app"]);
    fixture.assert_clean();

    let app = app_module(&fixture, &hir);
    let holder = item_in_module(&hir, &fixture, app, DefKind::Struct, "Holder");
    let secret = item_in_module(&hir, &fixture, app, DefKind::Struct, "Secret");
    let adt = hir.adt(holder).expect("the struct has an aggregate");
    let fields = hir.ty_ctxt().adt(adt).fields.clone();
    let field = |name: &str| -> DefId {
        fields
            .iter()
            .find(|field| {
                field
                    .name
                    .is_some_and(|field| fixture.symbol(field) == name)
            })
            .map(|field| field.def)
            .unwrap_or_else(|| panic!("no field `{name}`"))
    };
    let shown = field("shown");
    let hidden = field("hidden");

    assert!(hir.public_items().contains(&holder));
    assert!(hir.public_items().contains(&shown));
    assert!(!hir.public_items().contains(&hidden));
    assert!(!hir.is_public_api(secret));
}
