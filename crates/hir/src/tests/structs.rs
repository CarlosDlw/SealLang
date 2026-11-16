//! Struct lowering, literals, field places, and declaration validation.

use seal_def::DefId;
use seal_ty::TyKind;

use super::support::{Fixture, function, lower_clean, tail};
use crate::{ExprKind, Hir, Projection};

fn struct_def(fixture: &Fixture, hir: &Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            data.kind == seal_def::DefKind::Struct && fixture.symbol(data.name) == name
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no struct named `{name}`"))
}

#[test]
fn struct_fields_are_registered_in_the_type_context() {
    let (fixture, hir) = lower_clean("struct Point { x: i32, y: *i32 } func main() {}");
    let definition = struct_def(&fixture, &hir, "Point");
    let adt = hir.adt(definition).expect("the struct has an aggregate");
    let def = hir.ty_ctxt().adt(adt);
    assert_eq!(def.fields.len(), 2);
    assert_eq!(fixture.symbol(def.fields[0].name), "x");
    assert_eq!(fixture.symbol(def.fields[1].name), "y");
    assert!(matches!(
        hir.ty_ctxt().kind(def.fields[1].ty),
        TyKind::Ref { .. }
    ));
}

#[test]
fn struct_literals_resolve_fields_in_source_order() {
    let (fixture, hir) =
        lower_clean("struct Point { x: i32, y: i32 } func main() { Point { y: 1, x: 2 } }");
    let (_, body) = function(&fixture, &hir, "main");
    let expr = tail(&hir, body);
    let ExprKind::Struct { adt, fields } = hir.expr(expr).kind.clone() else {
        panic!("expected a struct literal");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(adt)), "Point");
    assert_eq!(fields.len(), 2);
    assert_eq!(fixture.symbol(hir.definitions().name(fields[0].0)), "y");
    assert_eq!(fixture.symbol(hir.definitions().name(fields[1].0)), "x");
}

#[test]
fn field_access_lowers_to_a_projected_place() {
    let (fixture, hir) =
        lower_clean("struct Point { x: i32 } func main() -> i32 { let p = Point { x: 1 }; p.x }");
    let (_, body) = function(&fixture, &hir, "main");
    let expr = tail(&hir, body);
    assert!(matches!(hir.expr(expr).kind, ExprKind::Field { .. }));
    let place = hir.place(expr).expect("field access is a place");
    assert!(
        matches!(place.projections(), [Projection::Field(name)] if fixture.symbol(*name) == "x")
    );
}

#[test]
fn a_field_of_a_temporary_is_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } func main() { Point { x: 1 }.x }");
    let _ = fixture.lower();
    fixture.expect("E2012", "requires a place");
}

#[test]
fn unknown_and_repeated_fields_are_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } func main() { Point { x: 1, z: 2 } }");
    let _ = fixture.lower();
    fixture.expect("E2007", "no field");

    let fixture = Fixture::new("struct Point { x: i32 } func main() { Point { x: 1, x: 2 } }");
    let _ = fixture.lower();
    fixture.expect("E2008", "specified twice");
}

#[test]
fn missing_fields_are_reported_once() {
    let fixture = Fixture::new("struct Point { x: i32, y: i32 } func main() { Point { x: 1 } }");
    let _ = fixture.lower();
    fixture.expect("E2009", "missing fields");
}

#[test]
fn type_names_are_not_values_and_values_are_not_types() {
    let fixture = Fixture::new("struct Point { x: i32 } func main() -> i32 { Point }");
    let _ = fixture.lower();
    fixture.expect("E2010", "type");

    let fixture = Fixture::new("func helper() {} func f(p: helper) {}");
    let _ = fixture.lower();
    fixture.expect("E2011", "value");
}

#[test]
fn unknown_types_are_reported_during_lowering() {
    let fixture = Fixture::new("func f(p: Missing) {}");
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find type");
}

#[test]
fn duplicate_structs_and_fields_are_rejected() {
    let fixture = Fixture::new("struct A { x: i32 } struct A { y: i32 } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2005", "defined twice");

    let fixture = Fixture::new("struct A { x: i32, x: i32 } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2006", "declared twice");
}

#[test]
fn recursive_by_value_types_are_rejected() {
    let fixture = Fixture::new("struct Node { next: Node } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E3016", "recursive");
}

#[test]
fn recursion_through_a_reference_is_accepted() {
    lower_clean("struct Node { next: *Node } func main() {}");
    lower_clean("struct A { b: B } struct B { a: *A } func main() {}");
}

#[test]
fn unsized_fields_are_rejected() {
    let fixture = Fixture::new("struct Bad { text: str } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E3010", "unsized");
}
