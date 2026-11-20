//! Packages, modules, imports, and the public API.

use seal_def::{DefId, DefKind};

use crate::tests::support::{Fixture, lower_package_clean};
use crate::{Hir, ModuleId};

fn named(hir: &Hir, fixture: &Fixture, module: ModuleId, name: &str) -> DefId {
    hir.items_in_module(module)
        .into_iter()
        .find(|def| {
            hir.definitions().kind(*def) == DefKind::Function
                && fixture.symbol(hir.definitions().name(*def)) == name
        })
        .unwrap_or_else(|| panic!("no function named `{name}` in the module"))
}

fn child(hir: &Hir, fixture: &Fixture, parent: ModuleId, name: &str) -> ModuleId {
    hir.child_module(parent, fixture.interner.intern(name))
        .unwrap_or_else(|| panic!("no submodule named `{name}`"))
}

#[test]
fn a_file_without_a_declaration_belongs_to_the_package_root() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "use app::util::{double, extra};\n\
             func main() -> i32 { double(20) + extra(1) }",
        ),
        (
            "util.sl",
            "module app::util;\n\
             internal func double(value: i32) -> i32 { value * 2 }",
        ),
        (
            "fragment.sl",
            "module app::util;\n\
             internal func extra(value: i32) -> i32 { value + 1 }",
        ),
    ]);

    let root = hir.package(hir.primary_package()).root;
    let main = named(&hir, &fixture, root, "main");
    assert!(hir.module(hir.module_of_def(main)).path.is_root());
    assert_eq!(hir.package_of_def(main), hir.primary_package());
    assert_eq!(hir.packages().len(), 1);
}

#[test]
fn one_module_merges_its_fragments() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "use app::util::{double, extra};\n\
             func main() -> i32 { double(20) + extra(1) }",
        ),
        (
            "util.sl",
            "module app::util;\n\
             internal func double(value: i32) -> i32 { value * 2 }",
        ),
        (
            "fragment.sl",
            "module app::util;\n\
             internal func extra(value: i32) -> i32 { value + 1 }",
        ),
    ]);

    let root = hir.package(hir.primary_package()).root;
    let util = child(&hir, &fixture, child(&hir, &fixture, root, "app"), "util");
    assert_eq!(hir.module(util).path.display(&fixture.interner), "app.util");
    assert!(hir.module(util).path.starts_with(&hir.module(util).path));

    let double = named(&hir, &fixture, util, "double");
    let extra = named(&hir, &fixture, util, "extra");
    assert_eq!(hir.module_of_def(double), util);
    assert_eq!(hir.module_of_def(extra), util);
    assert_eq!(hir.items_in_module(util).len(), 2);
}

#[test]
fn a_private_item_cannot_be_imported_from_another_module() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use app::secret;\nfunc main() -> i32 { secret() }",
        ),
        ("app.sl", "module app;\nprivate func secret() -> i32 { 1 }"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2072", "private");
}

#[test]
fn an_unknown_module_is_reported() {
    let fixture = Fixture::package(&[("main.sl", "use nope::value;\nfunc main() {}")]);
    let _ = fixture.lower_package();
    fixture.expect("E2073", "cannot find the module or dependency `nope`");
}

#[test]
fn a_missing_item_in_an_imported_module_is_reported() {
    let fixture = Fixture::package(&[
        ("main.sl", "use app::missing;\nfunc main() {}"),
        ("app.sl", "module app;\ninternal func present() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2073", "cannot find `missing`");
}

#[test]
fn two_fragments_cannot_declare_the_same_name() {
    let fixture = Fixture::package(&[
        ("first.sl", "module app;\nfunc same() {}"),
        ("second.sl", "module app;\nfunc same() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2083", "the function `same` is defined twice");
}

#[test]
fn a_public_item_outside_the_api_is_rejected() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use app::answer;\nfunc main() -> i32 { answer() }",
        ),
        ("app.sl", "module app;\npublic func answer() -> i32 { 42 }"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2080", "the public item `answer` is not reachable");
}

#[test]
fn an_exposed_module_publishes_its_public_items() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use app::answer;\nfunc main() -> i32 { answer() }",
        ),
        ("app.sl", "module app;\npublic func answer() -> i32 { 42 }"),
    ]);
    let hir = fixture.lower_package_exposing(&["app"]);
    fixture.assert_clean();

    let root = hir.package(hir.primary_package()).root;
    let app = child(&hir, &fixture, root, "app");
    let answer = named(&hir, &fixture, app, "answer");
    assert!(hir.is_public_api(answer));
    assert!(hir.public_items().contains(&answer));
    assert!(hir.is_visible_from(answer, root));
}

#[test]
fn a_module_is_not_a_value() {
    let fixture = Fixture::package(&[
        ("main.sl", "func main() { let value = app; }"),
        ("app.sl", "module app;\ninternal func helper() {}"),
    ]);
    let _ = fixture.lower_package();
    fixture.expect("E2086", "`app` is a module, not a value");
}

#[test]
fn a_type_declared_in_one_fragment_is_usable_from_another() {
    let (fixture, hir) = lower_package_clean(&[
        (
            "main.sl",
            "use shapes::Point;\n\
             func main() -> i32 { let point = Point { x: 20, y: 22 }; point.x + point.y }",
        ),
        (
            "shapes.sl",
            "module shapes;\n\
             internal struct Point { x: i32, y: i32 }",
        ),
    ]);

    let root = hir.package(hir.primary_package()).root;
    let shapes = child(&hir, &fixture, root, "shapes");
    let point = hir
        .items_in_module(shapes)
        .into_iter()
        .find(|def| hir.definitions().kind(*def) == DefKind::Struct)
        .expect("the struct is declared in `shapes`");
    assert_eq!(hir.module_of_def(point), shapes);
    assert!(hir.adt(point).is_some());
}
