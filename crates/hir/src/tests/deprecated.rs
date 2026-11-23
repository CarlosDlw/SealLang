//! Deprecation metadata is copied from declarations into definitions.

use seal_def::{DefId, DefKind};
use seal_diagnostics::Level;

use crate::CfgEnvironment;
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

/// Returns the deprecation warnings collected by a fixture.
fn deprecated_warnings(fixture: &Fixture) -> Vec<seal_diagnostics::Diagnostic> {
    fixture
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.level() == Level::Warning
                && diagnostic.lint() == Some(seal_diagnostics::DEPRECATED)
        })
        .collect()
}

#[test]
fn every_resolved_hir_use_warns_at_its_use_span() {
    let fixture = Fixture::new(
        "deprecated(since: \"0.2.0\", note: \"use new\", replacement: \"new\")\n\
         func old() {}\n\
         deprecated(since: \"0.3.0\")\n\
         struct Old { deprecated(since: \"0.3.1\") field: i32 }\n\
         deprecated(since: \"0.4.0\")\n\
         trait OldTrait {}\n\
         impl OldTrait for Old {}\n\
         deprecated(since: \"0.5.0\")\n\
         enum Choice { deprecated(since: \"0.5.1\") Old { deprecated(since: \"0.5.2\") value: i32 }, New }\n\
         func main() {\n\
           old();\n\
           let value: Old = Old { field: 1 };\n\
           match Choice::Old { value: 1 } {\n\
             Choice::Old { value } => { let _ = value; }\n\
             Choice::New => { let _ = value; }\n\
           }\n\
         }",
    );
    let _ = fixture.lower();
    let warnings = deprecated_warnings(&fixture);
    let messages: Vec<&str> = warnings
        .iter()
        .map(seal_diagnostics::Diagnostic::message)
        .collect();
    assert!(
        messages.iter().any(|message| message.contains("`old`")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("`Old`")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("`OldTrait`")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("`Choice`")),
        "{messages:?}"
    );
    assert!(
        warnings.iter().any(|warning| warning
            .message()
            .contains("`Old` is deprecated since 0.5.1")),
        "the variant warning is present: {messages:?}"
    );
    assert!(
        warnings.iter().any(|warning| {
            warning.message().contains("`field`") || warning.message().contains("`value`")
        }),
        "a field warning is present: {messages:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.helps().iter().any(|help| help == "use new")),
        "the replacement help is present"
    );
}

#[test]
fn a_use_inside_its_own_deprecated_declaration_does_not_warn() {
    let fixture = Fixture::new(
        "deprecated(since: \"0.2.0\") func old() { old(); }\n\
         deprecated(since: \"0.3.0\") struct Node { next: *Node }",
    );
    let _ = fixture.lower();
    assert!(deprecated_warnings(&fixture).is_empty());
}

#[test]
fn a_cfg_inactive_use_does_not_warn() {
    let fixture = Fixture::package(&[(
        "main.sl",
        "deprecated(since: \"0.2.0\") func old() {}\n\
         cfg(feature(\"extra\")) func hidden() { old(); }\n\
         func main() {}",
    )]);
    let _ = fixture.lower_package_with_cfg(CfgEnvironment::default());
    assert!(deprecated_warnings(&fixture).is_empty());
}

#[test]
fn imports_and_reexports_warn_at_the_import_path() {
    let fixture = Fixture::package(&[
        (
            "main.sl",
            "use api::old;\npublic use api::old as renamed;\nfunc main() {}",
        ),
        (
            "api.sl",
            "module api;\npublic deprecated(since: \"0.2.0\") func old() {}",
        ),
    ]);
    let _ = fixture.lower_package_exposing(&["api", "root"]);
    let warnings = deprecated_warnings(&fixture);
    assert_eq!(
        warnings.len(),
        2,
        "one warning per import and re-export: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .all(|warning| warning.message().contains("`old`"))
    );
}

#[test]
fn definitions_without_a_modifier_have_no_deprecation() {
    let (fixture, hir) = lower_clean("func current() {}\nstruct S { field: i32 }");
    let function = definition(&fixture, &hir, DefKind::Function, "current");
    assert!(hir.definitions().get(function).deprecated.is_none());
    let field = definition(&fixture, &hir, DefKind::Field, "field");
    assert!(hir.definitions().get(field).deprecated.is_none());
}
