//! Deprecation metadata is copied from declarations into definitions.

use seal_def::{DefId, DefKind};

use crate::Hir;
use crate::tests::support::{Fixture, lower_clean};

fn definition(fixture: &Fixture, hir: &Hir, kind: DefKind, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

#[test]
fn items_fields_variants_and_trait_methods_record_deprecation() {
    let (fixture, hir) = lower_clean(
        "deprecated(since: \"0.2.0\", note: \"use new\", replacement: \"new\")\n\
         func old_fn() {}\n\
         struct S { deprecated(since: \"0.3.0\") old: i32 }\n\
         enum E { deprecated(since: \"0.4.0\") Old }\n\
         trait T { deprecated(since: \"0.5.0\") func old_trait(); }\n\
         deprecated(since: \"0.6.0\") const OLD_CONST: i32 = 1;\n\
         deprecated(since: \"0.7.0\") static OLD_STATIC: i32 = 2;\n\
         impl S { deprecated(since: \"0.8.0\") func old_method() {} }\n\
         extern \"c\" { deprecated(since: \"0.9.0\") func old_extern(); }",
    );

    let function = definition(&fixture, &hir, DefKind::Function, "old_fn");
    let deprecation = hir
        .definitions()
        .get(function)
        .deprecated
        .expect("the function is deprecated");
    assert_eq!(fixture.symbol(deprecation.since), "0.2.0");
    assert_eq!(fixture.symbol(deprecation.note.expect("note")), "use new");
    assert_eq!(
        fixture.symbol(deprecation.replacement.expect("replacement")),
        "new"
    );

    let field = definition(&fixture, &hir, DefKind::Field, "old");
    assert_eq!(
        fixture.symbol(
            hir.definitions()
                .get(field)
                .deprecated
                .expect("the field is deprecated")
                .since
        ),
        "0.3.0"
    );

    let variant = definition(&fixture, &hir, DefKind::Variant, "Old");
    assert_eq!(
        fixture.symbol(
            hir.definitions()
                .get(variant)
                .deprecated
                .expect("the variant is deprecated")
                .since
        ),
        "0.4.0"
    );

    let method = definition(&fixture, &hir, DefKind::TraitMethod, "old_trait");
    assert_eq!(
        fixture.symbol(
            hir.definitions()
                .get(method)
                .deprecated
                .expect("the trait method is deprecated")
                .since
        ),
        "0.5.0"
    );

    for (kind, name, since) in [
        (DefKind::Const, "OLD_CONST", "0.6.0"),
        (DefKind::Static, "OLD_STATIC", "0.7.0"),
        (DefKind::Function, "old_method", "0.8.0"),
        (DefKind::ExternFunction, "old_extern", "0.9.0"),
    ] {
        let definition = definition(&fixture, &hir, kind, name);
        assert_eq!(
            fixture.symbol(
                hir.definitions()
                    .get(definition)
                    .deprecated
                    .unwrap_or_else(|| panic!("`{name}` is not deprecated"))
                    .since
            ),
            since
        );
    }
}

#[test]
fn definitions_without_a_modifier_have_no_deprecation() {
    let (fixture, hir) = lower_clean("func current() {}\nstruct S { field: i32 }");
    let function = definition(&fixture, &hir, DefKind::Function, "current");
    assert!(hir.definitions().get(function).deprecated.is_none());
    let field = definition(&fixture, &hir, DefKind::Field, "field");
    assert!(hir.definitions().get(field).deprecated.is_none());
}
