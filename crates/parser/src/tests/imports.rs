//! Absolute imports, groups, aliases, and their recovery.

use seal_ast::{ItemKind, UseRoot, UseSegment};

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

fn sole_use(ast: &seal_ast::Ast) -> &seal_ast::UseDecl {
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::Use(declaration) = &item.kind else {
        panic!("expected an import, got {:?}", item.kind);
    };
    declaration
}

#[test]
fn root_imports_start_at_the_package_root() {
    let (fixture, ast) = parse_clean("use root::C;");
    let declaration = sole_use(&ast);
    assert_eq!(declaration.path.root, UseRoot::Root);
    assert_eq!(fixture.text(declaration.path.root_span), "root");
    let UseSegment::Name { name, span } = &declaration.path.segments[0] else {
        panic!("expected a named segment");
    };
    assert_eq!(fixture.symbol(*name), "C");
    assert_eq!(fixture.text(*span), "C");
    assert_eq!(fixture.text(declaration.path.span), "root::C");
    assert_invariants(&fixture, &ast);
}

#[test]
fn absolute_imports_keep_every_segment() {
    let (fixture, ast) = parse_clean("use geometry::shapes::Circle;");
    let declaration = sole_use(&ast);
    let UseRoot::Name(root) = declaration.path.root else {
        panic!("expected a named root");
    };
    assert_eq!(fixture.symbol(root), "geometry");
    let names: Vec<String> = declaration
        .path
        .segments
        .iter()
        .map(|segment| match segment {
            UseSegment::Name { name, .. } => fixture.symbol(*name),
            UseSegment::Group { .. } => panic!("unexpected group"),
        })
        .collect();
    assert_eq!(names, ["shapes", "Circle"]);
}

#[test]
fn import_groups_accept_aliases_and_a_trailing_comma() {
    let (fixture, ast) = parse_clean("use geometry::{Circle, Polygon as Poly,};");
    let declaration = sole_use(&ast);
    let UseSegment::Group { items, .. } = &declaration.path.segments[0] else {
        panic!("expected an import group");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(fixture.symbol(items[0].name), "Circle");
    assert_eq!(fixture.symbol(items[1].name), "Polygon");
    assert_eq!(fixture.symbol(items[1].alias.expect("alias")), "Poly");
    assert_eq!(fixture.text(items[1].span), "Polygon as Poly");
    assert_eq!(
        fixture.text(items[1].alias_span.expect("alias span")),
        "Poly"
    );
}

#[test]
fn a_whole_path_alias_is_recorded_with_its_own_span() {
    let (fixture, ast) = parse_clean("use geometry::shapes as shapes;");
    let declaration = sole_use(&ast);
    assert_eq!(declaration.path.segments.len(), 1);
    assert_eq!(
        fixture.symbol(declaration.path.alias.expect("alias")),
        "shapes"
    );
    assert_eq!(
        fixture.text(declaration.path.alias_span.expect("alias span")),
        "shapes"
    );
}

#[test]
fn glob_imports_are_rejected() {
    for source in ["use geometry::*;", "use geometry::shapes::*;"] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_eq!(fixture.codes(), ["E2076"], "source: {source}");
        assert!(matches!(
            ast.item(ast.program().items()[0]).kind,
            ItemKind::Use(_)
        ));
    }
}

#[test]
fn an_import_after_an_item_is_rejected_but_kept() {
    let fixture = Fixture::new("func first() {} use geometry::Circle;");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1001"]);
    assert_eq!(ast.program().items().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Use(_)
    ));
}

#[test]
fn root_is_reserved_after_the_first_segment() {
    let fixture = Fixture::new("use geometry::root::Circle;");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2075"]);
}

#[test]
fn imports_reject_visibility_and_test_modifiers() {
    for source in ["public use geometry::Circle;", "test use geometry::Circle;"] {
        let fixture = Fixture::new(source);
        fixture.parse();
        assert_eq!(fixture.codes(), ["E2079"], "source: {source}");
    }
}

#[test]
fn cfg_before_an_import_is_kept() {
    let (fixture, ast) = parse_clean("cfg(feature(\"geometry\")) use geometry::Circle;");
    let item = ast.item(ast.program().items()[0]);
    let cfg = item.modifiers.cfg.expect("cfg condition");
    assert!(matches!(
        ast.cfg_expr(cfg).kind,
        seal_ast::CfgExprKind::Feature(_)
    ));
    assert_invariants(&fixture, &ast);
}

#[test]
fn an_unclosed_import_group_reports_once_and_recovers() {
    let fixture = Fixture::new("use geometry::{ func next() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1002"]);
    assert_eq!(ast.program().items().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn a_missing_import_name_recovers_at_the_semicolon() {
    let fixture = Fixture::new("use geometry::; func next() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1005"]);
    assert_eq!(ast.program().items().len(), 2);
}

#[test]
fn dot_paths_still_parse_as_field_access() {
    let (fixture, ast) = parse_clean("func f() { value.field }");
    assert_invariants(&fixture, &ast);
}
