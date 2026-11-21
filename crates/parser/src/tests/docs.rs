//! Doc-comment attachment.

use seal_ast::{ItemKind, VariantFieldList};

use crate::tests::support::{Fixture, parse_clean};

fn item(ast: &seal_ast::Ast, index: usize) -> &seal_ast::Item {
    ast.item(ast.program().items()[index])
}

#[test]
fn outer_docs_attach_to_items_modifiers_fields_and_variants() {
    let source = "\
//! The module.
module app::util;

/// A point.
/// It has two fields.
public struct Point {
    /// The x coordinate.
    x: i32,
    y: i32,
}

/** A mode. */
internal enum Mode {
    /// The on state.
    On,
    Off,
}
";
    let (fixture, ast) = parse_clean(source);
    let module = item(&ast, 0);
    let ItemKind::Module(declaration) = &module.kind else {
        panic!("first item is not a module");
    };
    assert_eq!(
        fixture.text(declaration.docs.expect("module docs")),
        "//! The module."
    );

    let point = item(&ast, 1);
    assert_eq!(
        fixture.text(point.docs.expect("item docs")),
        "/// A point.\n/// It has two fields."
    );
    let ItemKind::Struct(point) = &point.kind else {
        panic!("second item is not a struct");
    };
    let x = ast.field(point.fields[0]);
    assert_eq!(
        fixture.text(x.docs.expect("field docs")),
        "/// The x coordinate."
    );
    assert!(ast.field(point.fields[1]).docs.is_none());

    let mode = item(&ast, 2);
    assert_eq!(
        fixture.text(mode.docs.expect("block item docs")),
        "/** A mode. */"
    );
    let ItemKind::Enum(mode) = &mode.kind else {
        panic!("third item is not an enum");
    };
    let on = ast.variant(mode.variants[0]);
    assert_eq!(
        fixture.text(on.docs.expect("variant docs")),
        "/// The on state."
    );
    assert!(ast.variant(mode.variants[1]).docs.is_none());
    assert!(matches!(on.fields, VariantFieldList::Unit));
}

#[test]
fn comments_inside_a_body_do_not_leak_to_the_next_item() {
    let source = "\
func first() -> i32 {
    // A body comment.
    let value = 1;
    value
}

func second() -> i32 { 2 }
";
    let (fixture, ast) = parse_clean(source);
    assert!(item(&ast, 0).docs.is_none());
    assert!(item(&ast, 1).docs.is_none());
    let _ = fixture;
}

#[test]
fn an_ordinary_comment_breaks_the_doc_attachment() {
    let source = "\
/// This belongs to nothing.
// A plain comment between the docs and the item.
func value() -> i32 { 1 }
";
    let (_fixture, ast) = parse_clean(source);
    assert!(item(&ast, 0).docs.is_none());
}

#[test]
fn fixture_text_is_available_for_docs() {
    let fixture = Fixture::new("/// doc\nfunc value() {}\n");
    let (_, ast) = parse_clean("/// doc\nfunc value() {}\n");
    assert_eq!(fixture.text(item(&ast, 0).docs.expect("docs")), "/// doc");
}
