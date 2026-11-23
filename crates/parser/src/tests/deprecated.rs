//! The contextual `deprecated(...)` declaration modifier.

use seal_ast::{ItemKind, VariantFieldList};

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

#[test]
fn deprecation_metadata_is_parsed_on_functions() {
    let (fixture, ast) = parse_clean(
        "deprecated(since: \"0.2.0\", note: \"use new_api\", replacement: \"new_api\")\n\
         func old_api() {}",
    );
    let item = ast.item(ast.program().items()[0]);
    let deprecation = item.modifiers.deprecated.expect("deprecation is recorded");
    assert_eq!(fixture.symbol(deprecation.since), "0.2.0");
    assert_eq!(
        deprecation.note.map(|note| fixture.symbol(note)),
        Some("use new_api".to_owned())
    );
    assert_eq!(
        deprecation
            .replacement
            .map(|replacement| fixture.symbol(replacement)),
        Some("new_api".to_owned())
    );
    assert_eq!(
        fixture.text(deprecation.span),
        "deprecated(since: \"0.2.0\", note: \"use new_api\", replacement: \"new_api\")"
    );
    assert_invariants(&fixture, &ast);
}

#[test]
fn since_is_the_only_required_entry() {
    let (fixture, ast) = parse_clean("deprecated(since: \"1.0.0\") func old() {}");
    let deprecation = ast
        .item(ast.program().items()[0])
        .modifiers
        .deprecated
        .expect("deprecation is recorded");
    assert_eq!(fixture.symbol(deprecation.since), "1.0.0");
    assert!(deprecation.note.is_none());
    assert!(deprecation.replacement.is_none());
    assert_invariants(&fixture, &ast);
}

#[test]
fn fields_variants_and_trait_methods_carry_deprecation() {
    let (fixture, ast) = parse_clean(
        "struct S { deprecated(since: \"0.2.0\") old: i32 }\n\
         enum E {\n\
             deprecated(since: \"0.2.0\") Old,\n\
             New { deprecated(since: \"0.3.0\") value: i32 },\n\
         }\n\
         trait T { deprecated(since: \"0.4.0\") func old(); }",
    );
    let items = ast.program().items();
    let ItemKind::Struct(declaration) = &ast.item(items[0]).kind else {
        panic!("expected a struct");
    };
    assert!(ast.field(declaration.fields[0]).deprecated.is_some());

    let ItemKind::Enum(declaration) = &ast.item(items[1]).kind else {
        panic!("expected an enum");
    };
    assert!(ast.variant(declaration.variants[0]).deprecated.is_some());
    let VariantFieldList::Named(fields) = &ast.variant(declaration.variants[1]).fields else {
        panic!("expected named variant fields");
    };
    let field = ast.field(fields[0]);
    assert!(field.deprecated.is_some());
    assert_eq!(fixture.symbol(field.name), "value");

    let ItemKind::Trait(declaration) = &ast.item(items[2]).kind else {
        panic!("expected a trait");
    };
    let ItemKind::TraitMethod(method) = &ast.item(declaration.methods[0]).kind else {
        panic!("expected a trait method");
    };
    let deprecation = method.deprecated.expect("deprecation is recorded");
    assert_eq!(fixture.symbol(deprecation.since), "0.4.0");
    assert_invariants(&fixture, &ast);
}

#[test]
fn impl_methods_carry_deprecation() {
    let (fixture, ast) = parse_clean(
        "struct S {}\n\
         impl S { deprecated(since: \"0.2.0\") func old() {} }",
    );
    let ItemKind::Impl(implementation) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected an impl");
    };
    assert!(
        ast.item(implementation.functions[0])
            .modifiers
            .deprecated
            .is_some()
    );
    assert_invariants(&fixture, &ast);
}

#[test]
fn deprecated_remains_an_ordinary_identifier() {
    let (fixture, ast) = parse_clean(
        "func deprecated() {}\n\
         struct S { deprecated: i32 }\n\
         func use() { let deprecated = 1; }",
    );
    let ItemKind::Struct(declaration) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected a struct");
    };
    assert_eq!(
        fixture.symbol(ast.field(declaration.fields[0]).name),
        "deprecated"
    );
    assert_invariants(&fixture, &ast);
}

#[test]
fn malformed_deprecations_are_rejected() {
    for (source, fragment) in [
        (
            "deprecated(note: \"old\") func f() {}",
            "requires a `since`",
        ),
        (
            "deprecated(since: \"0.2\") func f() {}",
            "is not a semantic version",
        ),
        (
            "deprecated(since: \"0.2.0\", since: \"0.3.0\") func f() {}",
            "`since` is written twice",
        ),
        (
            "deprecated(since: \"0.2.0\", mood: \"x\") func f() {}",
            "is not a deprecation entry",
        ),
        (
            "deprecated(since: \"0.2.0\", note: \"\") func f() {}",
            "cannot be empty",
        ),
        (
            "deprecated(since: \"0.2.0\", note: 3) func f() {}",
            "needs a string value",
        ),
        (
            "deprecated(since: \"0.2.0\") deprecated(since: \"0.3.0\") func f() {}",
            "only one `deprecated` modifier",
        ),
    ] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        fixture.expect(codes::INVALID_DEPRECATION, fragment);
        assert_eq!(
            fixture.codes(),
            [codes::INVALID_DEPRECATION],
            "source: {source}"
        );
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn control_characters_are_rejected() {
    let fixture = Fixture::new("deprecated(since: \"0.2.0\", note: \"line\\nbreak\") func f() {}");
    let ast = fixture.parse();
    fixture.expect(codes::INVALID_DEPRECATION, "control characters");
    assert_invariants(&fixture, &ast);
}

#[test]
fn overlong_text_is_rejected() {
    let note = "x".repeat(257);
    let source = format!("deprecated(since: \"0.2.0\", note: \"{note}\") func f() {{}}");
    let fixture = Fixture::new(&source);
    let ast = fixture.parse();
    fixture.expect(codes::INVALID_DEPRECATION, "at most 256 bytes");
    assert_invariants(&fixture, &ast);
}

#[test]
fn deprecation_is_rejected_on_modules_imports_impls_and_extern_blocks() {
    for source in [
        "deprecated(since: \"0.2.0\") module app",
        "deprecated(since: \"0.2.0\") use foo;",
        "deprecated(since: \"0.2.0\") impl S {}",
        "deprecated(since: \"0.2.0\") extern \"c\" { func f(); }",
    ] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert!(
            fixture.codes().iter().any(|code| code == "E2079"),
            "source: {source}, codes: {:?}",
            fixture.codes()
        );
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn positional_fields_cannot_be_deprecated() {
    let fixture = Fixture::new("enum E { V(deprecated(since: \"0.2.0\") i32) }");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2079"]);
    assert_invariants(&fixture, &ast);
}

#[test]
fn every_deprecatable_item_form_accepts_the_modifier() {
    let (fixture, ast) = parse_clean(
        "deprecated(since: \"0.1.0\") const C: i32 = 1;\n\
         deprecated(since: \"0.1.0\") static S: i32 = 1;\n\
         deprecated(since: \"0.1.0\") struct St {}\n\
         deprecated(since: \"0.1.0\") enum En { A }\n\
         deprecated(since: \"0.1.0\") trait Tr {}\n\
         extern \"c\" { deprecated(since: \"0.1.0\") func ext(); }",
    );
    for item in ast.program().items().iter().take(5) {
        assert!(
            ast.item(*item).modifiers.deprecated.is_some(),
            "item is not deprecated: {:?}",
            ast.item(*item).kind
        );
    }
    let ItemKind::ExternBlock(block) = &ast.item(ast.program().items()[5]).kind else {
        panic!("expected an extern block");
    };
    assert!(
        ast.item(block.items[0]).modifiers.deprecated.is_some(),
        "the external declaration is not deprecated"
    );
    assert_invariants(&fixture, &ast);
}

mod codes {
    pub(super) const INVALID_DEPRECATION: &str = "E1001";
}
