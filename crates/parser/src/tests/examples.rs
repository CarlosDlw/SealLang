//! The language examples must parse into the expected shapes.

use std::path::PathBuf;

use seal_ast::{ExprKind, Literal, StmtKind};

use crate::tests::support::{assert_invariants, body, function, parse_clean, sole_function, tail};

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn all_examples_parse_without_diagnostics() {
    for (name, items) in [
        ("0001_main.sl", 1),
        ("0002_variables.sl", 1),
        ("0003_operators.sl", 1),
        ("004_conditionals.sl", 3),
        ("005_while_loop.sl", 2),
        ("007_runtime_panic.sl", 1),
        ("008_exclusive_references.sl", 2),
        ("010_slices.sl", 3),
        ("011_methods.sl", 1),
        ("012_user_methods.sl", 3),
        ("013_method_borrows.sl", 3),
        ("014_method_namespaces.sl", 5),
        ("015_traits.sl", 8),
        ("016_trait_receivers.sl", 5),
        ("017_enums.sl", 6),
        ("018_tuple_variants.sl", 8),
        ("019_match.sl", 3),
        ("020_match_advanced.sl", 8),
        ("021_match_slice.sl", 5),
        ("022_match_reference.sl", 6),
    ] {
        let source = example(name);
        let (fixture, ast) = parse_clean(&source);
        assert_eq!(
            ast.program().len(),
            items,
            "{name} declares {items} function(s)"
        );
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn entry_point_examples_define_a_parameterless_main_returning_i32() {
    for name in [
        "0001_main.sl",
        "0002_variables.sl",
        "0003_operators.sl",
        "007_runtime_panic.sl",
    ] {
        let source = example(name);
        let (fixture, ast) = parse_clean(&source);
        let declaration = sole_function(&ast);
        assert_eq!(fixture.symbol(declaration.name), "main", "{name}");
        assert_eq!(declaration.params.len(), 0, "{name}");
        assert!(declaration.ret.is_some(), "{name} declares i32");
    }
}

#[test]
fn main_example_returns_zero_as_its_tail() {
    let source = example("0001_main.sl");
    let (_, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    assert!(matches!(
        tail(&ast, declaration).kind,
        ExprKind::Lit(Literal::Int { value: 0, .. })
    ));
}

#[test]
fn variables_example_has_bindings_and_a_tail_name() {
    let source = example("0002_variables.sl");
    let (fixture, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 5);
    for statement in &block.statements {
        assert!(matches!(ast.stmt(*statement).kind, StmtKind::Let(_)));
    }
    let tail = tail(&ast, declaration);
    match tail.kind {
        ExprKind::Path(symbol) => assert_eq!(fixture.symbol(symbol), "c"),
        ref other => panic!("expected path tail, got {other:?}"),
    }
}

#[test]
fn operators_example_has_expression_statements_and_a_tail() {
    let source = example("0003_operators.sl");
    let (fixture, ast) = parse_clean(&source);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert!(block.statements.len() > 15);
    assert!(block.tail.is_some());
    let tail = tail(&ast, declaration);
    match tail.kind {
        ExprKind::Path(symbol) => assert_eq!(fixture.symbol(symbol), "b1"),
        ref other => panic!("expected path tail, got {other:?}"),
    }
    assert!(fixture.file.text().contains("+="));
}

#[test]
fn conditionals_example_uses_if_as_an_expression() {
    let source = example("004_conditionals.sl");
    let (fixture, ast) = parse_clean(&source);
    assert_invariants(&fixture, &ast);
    let classify = function(&ast, 0);
    assert_eq!(fixture.symbol(classify.name), "classify");
    assert_eq!(classify.params.len(), 1);
    assert!(matches!(tail(&ast, classify).kind, ExprKind::If { .. }));
}

#[test]
fn while_example_contains_loops_and_loop_control() {
    let source = example("005_while_loop.sl");
    let (fixture, ast) = parse_clean(&source);
    assert_invariants(&fixture, &ast);
    let sum = function(&ast, 0);
    assert_eq!(fixture.symbol(sum.name), "sum_odd_below");
    assert!(
        body(&ast, sum)
            .statements
            .iter()
            .any(|statement| matches!(ast.stmt(*statement).kind, StmtKind::While { .. }))
    );
    let text = fixture.file.text();
    assert!(text.contains("continue;"));
    assert!(text.contains("break;"));
}
