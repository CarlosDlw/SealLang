//! Type alias parsing.

use seal_ast::{ItemKind, PrimitiveType, TypeKind};

use super::support::{assert_invariants, parse_clean};

#[test]
fn a_simple_alias_names_its_target() {
    let (fixture, ast) = parse_clean("type Id = i32;");
    assert_invariants(&fixture, &ast);
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::TypeAlias(alias) = &item.kind else {
        panic!("expected a type alias, got {:?}", item.kind);
    };
    assert_eq!(fixture.symbol(alias.name), "Id");
    assert_eq!(alias.generics.params.len(), 0);
    assert!(alias.where_clause.predicates.is_empty());
    assert!(matches!(
        ast.ty(alias.ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
}

#[test]
fn generic_aliases_keep_parameters_and_nested_types() {
    let (fixture, ast) = parse_clean("type Pair<T> = (T, *[T]);");
    assert_invariants(&fixture, &ast);
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::TypeAlias(alias) = &item.kind else {
        panic!("expected a type alias");
    };
    assert_eq!(alias.generics.params.len(), 1);
    assert!(matches!(ast.ty(alias.ty).kind, TypeKind::Tuple(ref elements) if elements.len() == 2));
}

#[test]
fn visibility_cfg_and_deprecation_attach_to_an_alias() {
    let (fixture, ast) = parse_clean(
        r#"public cfg(target.os == "linux") deprecated(since: "0.1.0") type FileId = u64;"#,
    );
    assert_invariants(&fixture, &ast);
    let item = ast.item(ast.program().items()[0]);
    assert_eq!(item.modifiers.visibility, seal_ast::Visibility::Public);
    assert!(item.modifiers.cfg.is_some());
    assert!(item.modifiers.deprecated.is_some());
    assert!(matches!(item.kind, ItemKind::TypeAlias(_)));
}

#[test]
fn a_where_clause_is_retained_for_lowering() {
    let (fixture, ast) = parse_clean("type Pair<T> where T: Copy = (T, T);");
    assert_invariants(&fixture, &ast);
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::TypeAlias(alias) = &item.kind else {
        panic!("expected a type alias");
    };
    assert_eq!(alias.where_clause.predicates.len(), 1);
}

#[test]
fn missing_equals_recovers_with_a_diagnostic() {
    let fixture = super::support::Fixture::new("type Id i32;");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `=`");
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::TypeAlias(_)
    ));
}

#[test]
fn missing_semicolon_recovers_with_a_diagnostic() {
    let fixture = super::support::Fixture::new("type Id = i32");
    let ast = fixture.parse();
    fixture.expect("E1002", "ends with `;`");
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::TypeAlias(_)
    ));
}

#[test]
fn test_and_bench_are_not_valid_on_an_alias() {
    let fixture = super::support::Fixture::new("test type Id = i32;");
    let ast = fixture.parse();
    fixture.expect("E1001", "`test` applies only to functions");
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::TypeAlias(_)
    ));
}

#[test]
fn export_is_not_valid_on_an_alias() {
    let fixture = super::support::Fixture::new(r#"export "c" type Id = i32;"#);
    let ast = fixture.parse();
    fixture.expect("E1001", "`export` applies only to functions");
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::TypeAlias(_)
    ));
}

#[test]
fn unsafe_is_rejected_but_the_alias_is_still_parsed() {
    let fixture = super::support::Fixture::new("unsafe type Id = i32;");
    let ast = fixture.parse();
    fixture.expect("E3071", "`unsafe` applies only to functions");
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::TypeAlias(_)
    ));
}

#[test]
fn type_remains_an_identifier_away_from_item_start() {
    let (fixture, ast) = parse_clean("func type() -> i32 { 0 }");
    assert_invariants(&fixture, &ast);
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::Func(_)
    ));
}
