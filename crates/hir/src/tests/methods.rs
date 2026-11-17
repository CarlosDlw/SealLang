//! Inherent impl registration and associated-path resolution.

use seal_def::DefKind;
use seal_symbol::Symbol;

use crate::{ExprKind, ReceiverKind, Res};

use super::support::{Fixture, function, lower_clean, path_res, tail};

#[test]
fn impl_functions_are_registered_under_their_type() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32 } \
         impl Point { \
             func new(x: i32) -> Point { Point { x: x } } \
             func get(self: *Point) -> i32 { (*self).x } \
             func set(self: *mut Point, value: i32) { (*self).x = value; } \
             func take(self: Point) -> i32 { self.x } \
         }",
    );

    let point = type_named(&fixture, &hir, "Point");
    let new = method_named(&fixture, &hir, point, "new");
    let get = method_named(&fixture, &hir, point, "get");
    let set = method_named(&fixture, &hir, point, "set");
    let take = method_named(&fixture, &hir, point, "take");

    assert_eq!(hir.definitions().kind(new), DefKind::Function);
    assert_eq!(hir.definitions().get(new).parent, Some(point));
    assert_eq!(hir.receiver(new), None);
    assert_eq!(hir.receiver(get), Some(ReceiverKind::Shared));
    assert_eq!(hir.receiver(set), Some(ReceiverKind::Exclusive));
    assert_eq!(hir.receiver(take), Some(ReceiverKind::Value));
    assert_eq!(hir.bodies().len(), 4);
}

#[test]
fn receiver_parameters_are_visible_as_locals() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32 } \
         impl Point { func get(self: *Point) -> i32 { (*self).x } }",
    );
    let point = type_named(&fixture, &hir, "Point");
    let get = method_named(&fixture, &hir, point, "get");
    let body = hir.body_of(get).expect("method has a body");

    assert_eq!(body.params.len(), 1);
    assert_eq!(
        fixture.symbol(hir.definitions().name(body.params[0].def)),
        "self"
    );
    let ExprKind::Field { base, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a field access");
    };
    let ExprKind::Deref { operand } = hir.expr(base).kind.clone() else {
        panic!("expected a dereference");
    };
    let Res::Local(local) = path_res(&hir, operand) else {
        panic!("self must resolve to a local");
    };
    assert_eq!(local, body.params[0].def);
}

#[test]
fn qualified_paths_resolve_to_associated_functions() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32 } \
         impl Point { func origin() -> Point { Point { x: 0 } } } \
         func main() -> Point { Point::origin() }",
    );
    let point = type_named(&fixture, &hir, "Point");
    let origin = method_named(&fixture, &hir, point, "origin");
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    assert_eq!(path_res(&hir, callee), Res::Function(origin));
}

#[test]
fn a_method_cannot_be_called_through_a_qualified_path() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         impl Point { func get(self: *Point) -> i32 { (*self).x } } \
         func main() -> i32 { Point::get() }",
    );
    let _ = fixture.lower();
    fixture.expect("E2019", "`get` is a method");
}

#[test]
fn a_missing_associated_function_is_reported() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } impl Point {} func main() -> i32 { Point::missing() }",
    );
    let _ = fixture.lower();
    fixture.expect("E2018", "has no associated function `missing`");
}

#[test]
fn duplicate_impl_functions_are_reported() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         impl Point { func get(self: *Point) -> i32 { (*self).x } } \
         impl Point { func get(self: *mut Point) -> i32 { (*self).x } }",
    );
    let hir = fixture.lower();
    fixture.expect("E2017", "defined twice");
    assert_eq!(
        hir.definitions()
            .iter()
            .filter(|data| data.kind == DefKind::Function)
            .count(),
        2
    );
}

#[test]
fn invalid_receivers_are_reported() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         impl Point { func get(self: i32) -> i32 { 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2016", "not a valid receiver");
}

#[test]
fn self_must_be_the_first_parameter() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         impl Point { func get(value: i32, self: *Point) -> i32 { value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2016", "`self` must be the first parameter");
}

#[test]
fn impl_targets_must_be_structs() {
    let fixture = Fixture::new("func helper() {} impl helper { func call() {} }");
    let _ = fixture.lower();
    fixture.expect("E2015", "is a value, not a type");

    let fixture = Fixture::new("impl Missing { func call() {} }");
    let _ = fixture.lower();
    fixture.expect("E2015", "cannot find struct `Missing`");
}

fn type_named(fixture: &Fixture, hir: &crate::Hir, name: &str) -> seal_def::DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Struct && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no struct named `{name}`"))
}

fn method_named(
    fixture: &Fixture,
    hir: &crate::Hir,
    type_def: seal_def::DefId,
    name: &str,
) -> seal_def::DefId {
    let symbol: Symbol = fixture.interner.intern(name);
    hir.method(type_def, symbol)
        .unwrap_or_else(|| panic!("no impl function named `{name}`"))
}
