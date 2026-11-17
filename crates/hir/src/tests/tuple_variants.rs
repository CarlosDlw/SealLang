//! Positional enum variant fields in HIR.

use seal_def::{DefId, DefKind};
use seal_ty::{TyKind, VariantIdx};

use super::support::{Fixture, function, lower_clean, tail};
use crate::{ExprKind, Hir};

fn enum_def(fixture: &Fixture, hir: &Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Enum && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no enum named `{name}`"))
}

#[test]
fn positional_fields_are_registered_in_declaration_order() {
    let (fixture, hir) = lower_clean("enum Pair { Both(i32, *i32), Empty } func main() {}");
    let definition = enum_def(&fixture, &hir, "Pair");
    let adt = hir.adt(definition).expect("the enum has an aggregate");
    let data = hir.ty_ctxt().adt(adt);
    let both = &data.variants[0];
    assert!(both.is_tuple());
    assert_eq!(both.fields.len(), 2);
    assert!(both.fields.iter().all(|field| field.name.is_none()));
    assert_eq!(
        fixture.symbol(hir.definitions().name(both.fields[0].def)),
        "#0"
    );
    assert_eq!(
        fixture.symbol(hir.definitions().name(both.fields[1].def)),
        "#1"
    );
    assert_eq!(
        hir.definitions().get(both.fields[0].def).parent,
        Some(both.def)
    );
    assert!(matches!(
        hir.ty_ctxt().kind(both.fields[1].ty),
        TyKind::Ref { .. }
    ));
    assert!(!data.variants[1].is_tuple());
}

#[test]
fn a_positional_variant_call_lowers_in_argument_order() {
    let (fixture, hir) = lower_clean(
        "enum Pair { Both(i32, i32), Empty } func main() -> Pair { Pair::Both(20, 22) }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Variant {
        adt,
        variant,
        fields,
        ..
    } = hir.expr(tail(&hir, body)).kind.clone()
    else {
        panic!("expected a variant value");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(adt)), "Pair");
    assert_eq!(variant, VariantIdx::from_u32(0).expect("small index"));
    assert_eq!(fields.len(), 2);
    assert_eq!(fixture.symbol(hir.definitions().name(fields[0].0)), "#0");
    assert_eq!(fixture.symbol(hir.definitions().name(fields[1].0)), "#1");
}

#[test]
fn positional_variant_shape_errors_are_reported() {
    let fixture =
        Fixture::new("enum Pair { Both(i32, i32) } func main() { let p = Pair::Both(1); }");
    let _ = fixture.lower();
    fixture.expect("E2038", "takes 2 field(s)");

    let fixture =
        Fixture::new("enum Pair { Both(i32, i32) } func main() { let p = Pair::Both(1, 2, 3); }");
    let _ = fixture.lower();
    fixture.expect("E2038", "takes 2 field(s)");

    let fixture =
        Fixture::new("enum Mode { Set { x: i32 } } func main() { let m = Mode::Set(1); }");
    let _ = fixture.lower();
    fixture.expect("E2037", "named fields");

    let fixture = Fixture::new("enum Mode { On } func main() { let m = Mode::On(); }");
    let _ = fixture.lower();
    fixture.expect("E2039", "without parentheses");

    let fixture =
        Fixture::new("enum Pair { Both(i32) } func main() { let p = Pair::Both { value: 1 }; }");
    let _ = fixture.lower();
    fixture.expect("E2036", "positional fields");
}
