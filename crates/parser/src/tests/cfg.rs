//! Typed `cfg(...)` conditions and their precedence.

use seal_ast::{CfgCmpOp, CfgExprId, CfgExprKind, CfgLiteral, ItemKind};

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

fn item_cfg(ast: &seal_ast::Ast) -> CfgExprId {
    let item = ast.item(ast.program().items()[0]);
    assert!(matches!(item.kind, ItemKind::Func(_)));
    item.modifiers.cfg.expect("item has a cfg condition")
}

#[test]
fn a_target_comparison_keeps_both_operands_and_the_operator() {
    let (fixture, ast) = parse_clean("cfg(target.os == \"linux\") func f() {}");
    let cfg = item_cfg(&ast);
    let expression = ast.cfg_expr(cfg);
    let CfgExprKind::Compare { lhs, op, rhs } = &expression.kind else {
        panic!("expected a comparison, got {:?}", expression.kind);
    };
    assert_eq!(*op, CfgCmpOp::Eq);
    assert!(matches!(ast.cfg_expr(*lhs).kind, CfgExprKind::Target(_)));
    assert!(matches!(
        ast.cfg_expr(*rhs).kind,
        CfgExprKind::Literal(CfgLiteral::Str(_))
    ));
    assert_eq!(fixture.text(expression.span), "target.os == \"linux\"");
    assert_invariants(&fixture, &ast);
}

#[test]
fn and_binds_tighter_than_or() {
    let (_, ast) = parse_clean("cfg(test || feature(\"a\") && debug_assertions) func f() {}");
    let expression = ast.cfg_expr(item_cfg(&ast));
    let CfgExprKind::Or { rhs, .. } = &expression.kind else {
        panic!("expected `||` at the root, got {:?}", expression.kind);
    };
    assert!(matches!(ast.cfg_expr(*rhs).kind, CfgExprKind::And { .. }));
}

#[test]
fn negation_and_parentheses_keep_their_shape() {
    let (_, ast) =
        parse_clean("cfg((target.os == \"linux\") || !(feature(\"std\") && test)) func f() {}");
    let expression = ast.cfg_expr(item_cfg(&ast));
    let CfgExprKind::Or { lhs, rhs } = &expression.kind else {
        panic!("expected `||` at the root, got {:?}", expression.kind);
    };
    assert!(matches!(
        ast.cfg_expr(*lhs).kind,
        CfgExprKind::Compare { .. }
    ));
    assert!(matches!(ast.cfg_expr(*rhs).kind, CfgExprKind::Not(_)));
}

#[test]
fn every_supported_key_parses() {
    let sources = [
        "cfg(feature(\"std\")) func f() {}",
        "cfg(env(\"HOME\")) func f() {}",
        "cfg(target.os == \"linux\") func f() {}",
        "cfg(target.arch == \"x86_64\") func f() {}",
        "cfg(target.env != \"musl\") func f() {}",
        "cfg(target.pointer_width == 64) func f() {}",
        "cfg(debug_assertions) func f() {}",
        "cfg(test) func f() {}",
        "cfg(bench) func f() {}",
    ];
    for source in sources {
        let (fixture, ast) = parse_clean(source);
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn a_bare_string_is_not_a_configuration_key() {
    let fixture = Fixture::new("cfg(\"linux\") func f() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2084"]);
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn an_unknown_key_is_rejected() {
    let fixture = Fixture::new("cfg(foo) func f() {}");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2084"]);
}

#[test]
fn an_unknown_target_key_is_rejected() {
    let fixture = Fixture::new("cfg(target.vendor == \"acme\") func f() {}");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2084"]);
}

#[test]
fn feature_takes_exactly_one_string() {
    let fixture = Fixture::new("cfg(feature(\"a\", \"b\")) func f() {}");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E1002"]);
}

#[test]
fn a_missing_comparison_value_is_rejected() {
    let fixture = Fixture::new("cfg(target.os ==) func f() {}");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2084"]);
}

#[test]
fn a_malformed_comparison_recovers_at_the_next_item() {
    let fixture = Fixture::new("cfg(target.os =) func next() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1002"]);
    assert_eq!(ast.program().items().len(), 1);
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn cfg_on_an_import_keeps_the_condition() {
    let (fixture, ast) = parse_clean("cfg(feature(\"geometry\")) use geometry::Circle;");
    let item = ast.item(ast.program().items()[0]);
    assert!(matches!(item.kind, ItemKind::Use(_)));
    assert!(matches!(
        ast.cfg_expr(item.modifiers.cfg.expect("cfg")).kind,
        CfgExprKind::Feature(_)
    ));
    assert_invariants(&fixture, &ast);
}
