//! Enum lowering, variant construction, and declaration validation.

use seal_def::{DefId, DefKind};
use seal_ty::{TyKind, VariantIdx};

use super::support::{Fixture, function, lower_clean, tail};
use crate::{ExprKind, Hir, Res, StmtKind};

fn enum_def(fixture: &Fixture, hir: &Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Enum && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no enum named `{name}`"))
}

#[test]
fn variants_and_their_fields_are_registered() {
    let (fixture, hir) =
        lower_clean("enum Shape { Empty, Circle { radius: i32, } } func main() {}");
    let definition = enum_def(&fixture, &hir, "Shape");
    let adt = hir.adt(definition).expect("the enum has an aggregate");
    let data = hir.ty_ctxt().adt(adt);
    assert!(data.is_enum());
    assert_eq!(data.variants.len(), 2);
    assert_eq!(fixture.symbol(data.variants[0].name), "Empty");
    assert!(data.variants[0].is_unit());
    assert_eq!(fixture.symbol(data.variants[1].name), "Circle");
    assert_eq!(fixture.symbol(data.variants[1].fields[0].name), "radius");

    let circle = fixture.interner.intern("Circle");
    let index = hir
        .variant(definition, circle)
        .expect("the variant is registered by name");
    assert_eq!(index, VariantIdx::from_u32(1).expect("small index"));

    let field = hir.definitions().get(data.variants[1].fields[0].def);
    assert_eq!(field.kind, DefKind::Field);
    assert_eq!(field.parent, Some(data.variants[1].def));
    let variant = hir.definitions().get(data.variants[1].def);
    assert_eq!(variant.kind, DefKind::Variant);
    assert_eq!(variant.parent, Some(definition));
}

#[test]
fn unit_and_payload_variants_lower_to_variant_nodes() {
    let (fixture, hir) = lower_clean(
        "enum Shape { Empty, Circle { radius: i32 } } \
         func main() -> Shape { \
           let empty = Shape::Empty; \
           Shape::Circle { radius: 1 } \
         }",
    );
    let (_, body) = function(&fixture, &hir, "main");

    let StmtKind::Let { init, .. } = hir.stmt(hir.block(body.root).statements[0]).kind.clone()
    else {
        panic!("expected a let statement");
    };
    let init = init.expect("the binding is initialized");
    let ExprKind::Variant {
        variant, fields, ..
    } = hir.expr(init).kind.clone()
    else {
        panic!("expected a unit variant value");
    };
    assert_eq!(variant, VariantIdx::from_u32(0).expect("small index"));
    assert!(fields.is_empty());

    let ExprKind::Variant {
        adt,
        variant,
        fields,
    } = hir.expr(tail(&hir, body)).kind.clone()
    else {
        panic!("expected a payload variant value");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(adt)), "Shape");
    assert_eq!(variant, VariantIdx::from_u32(1).expect("small index"));
    assert_eq!(fields.len(), 1);
    assert_eq!(
        fixture.symbol(hir.definitions().name(fields[0].0)),
        "radius"
    );
}

#[test]
fn inherent_functions_on_enums_resolve() {
    let (fixture, hir) = lower_clean(
        "enum Mode { On, Off } \
         impl Mode { func on() -> Mode { Mode::On } } \
         func main() -> Mode { Mode::on() }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Call { callee, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a call");
    };
    assert!(matches!(
        hir.expr(callee).kind,
        ExprKind::Path(Res::Function(_))
    ));
}

#[test]
fn duplicate_variants_and_fields_are_rejected() {
    let fixture = Fixture::new("enum Mode { On, On } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2029", "declared twice");

    let fixture = Fixture::new("enum Mode { On { x: i32, x: i32 } } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2006", "declared twice");
}

#[test]
fn construction_requires_the_right_variant_shape() {
    let fixture = Fixture::new("enum Mode { On } func main() { let mode = Mode::On { x: 1 }; }");
    let _ = fixture.lower();
    fixture.expect("E2033", "has no fields");

    let fixture =
        Fixture::new("enum Mode { Set { x: i32 } } func main() { let mode = Mode::Set; }");
    let _ = fixture.lower();
    fixture.expect("E2034", "has fields");

    let fixture = Fixture::new("enum Mode { On } func main() { let mode = Mode::Off { x: 1 }; }");
    let _ = fixture.lower();
    fixture.expect("E2032", "no variant");

    let fixture =
        Fixture::new("struct Point { x: i32 } func main() { let p = Point::X { x: 1 }; }");
    let _ = fixture.lower();
    fixture.expect("E2031", "not an enum");

    let fixture = Fixture::new("enum Mode { On } func main() { let mode = Mode { }; }");
    let _ = fixture.lower();
    fixture.expect("E2035", "not a struct");
}

#[test]
fn variant_literals_validate_their_fields() {
    let fixture = Fixture::new(
        "enum Mode { Set { x: i32, y: i32 } } func main() { let m = Mode::Set { x: 1 }; }",
    );
    let _ = fixture.lower();
    fixture.expect("E2009", "missing fields");

    let fixture =
        Fixture::new("enum Mode { Set { x: i32 } } func main() { let m = Mode::Set { z: 1 }; }");
    let _ = fixture.lower();
    fixture.expect("E2007", "no field `z`");

    let fixture = Fixture::new(
        "enum Mode { Set { x: i32 } } func main() { let m = Mode::Set { x: 1, x: 2 }; }",
    );
    let _ = fixture.lower();
    fixture.expect("E2008", "specified twice");
}

#[test]
fn a_variant_cannot_share_a_name_with_an_associated_function() {
    let fixture = Fixture::new(
        "enum Mode { On } impl Mode { func On() -> Mode { Mode::On } } func main() {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2030", "same name");
}

#[test]
fn recursive_and_unsized_variant_fields_are_rejected() {
    let fixture = Fixture::new("enum Node { Leaf, Next { next: Node } } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E3016", "recursive");

    let fixture = Fixture::new("enum Bad { Text { text: str } } func main() {}");
    let _ = fixture.lower();
    fixture.expect("E3010", "unsized");
}

#[test]
fn enum_kinds_are_available_to_type_annotations() {
    let (fixture, hir) =
        lower_clean("enum Mode { On, Off } func first(mode: Mode) -> Mode { mode } func main() {}");
    let definition = enum_def(&fixture, &hir, "Mode");
    let adt = hir.adt(definition).expect("the enum has an aggregate");
    assert_eq!(
        hir.ty_ctxt().kind(hir.ty_ctxt().adt_ty(adt)),
        TyKind::Adt(adt)
    );
    assert!(hir.ty_ctxt().is_sized(hir.ty_ctxt().adt_ty(adt)));
}
