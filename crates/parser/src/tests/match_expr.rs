//! `match` expressions and the pattern grammar.

use seal_ast::{Ast, ExprKind, MatchArm, PatternKind, PatternLiteral, VariantPatternFields};

use crate::tests::support::{Fixture, assert_invariants, body, function, parse_clean};

const COMMANDS: &str = "\
enum Command {
  Stop,
  Move(i32),
  Write { code: i32 },
}

func classify(command: Command) -> i32 {
  match command {
    Command::Stop => 0,
    Command::Move(step) => step,
    Command::Write { code } => code,
  }
}
";

fn match_arms(ast: &Ast, index: usize) -> &[MatchArm] {
    let declaration = function(ast, index);
    let tail = body(ast, declaration)
        .tail
        .expect("the function has a tail expression");
    match &ast.expr(tail).kind {
        ExprKind::Match { arms, .. } => arms,
        other => panic!("expected a match expression, got {other:?}"),
    }
}

#[test]
fn a_match_expression_parses_patterns_fields_and_bodies() {
    let (fixture, ast) = parse_clean(COMMANDS);
    assert_invariants(&fixture, &ast);

    let arms = match_arms(&ast, 1);
    assert_eq!(arms.len(), 3);
    assert!(arms.iter().all(|arm| arm.guard.is_none()));

    let PatternKind::Variant {
        type_name,
        name,
        fields,
        ..
    } = &ast.pattern(arms[0].pattern).kind
    else {
        panic!("expected a variant pattern");
    };
    assert_eq!(fixture.symbol(*type_name), "Command");
    assert_eq!(fixture.symbol(*name), "Stop");
    assert!(matches!(fields, VariantPatternFields::Unit));

    let PatternKind::Variant { name, fields, .. } = &ast.pattern(arms[1].pattern).kind else {
        panic!("expected a variant pattern");
    };
    assert_eq!(fixture.symbol(*name), "Move");
    let VariantPatternFields::Positional(patterns) = fields else {
        panic!("expected positional field patterns");
    };
    assert_eq!(patterns.len(), 1);
    assert!(matches!(
        &ast.pattern(patterns[0]).kind,
        PatternKind::Binding { .. }
    ));

    let PatternKind::Variant { name, fields, .. } = &ast.pattern(arms[2].pattern).kind else {
        panic!("expected a variant pattern");
    };
    assert_eq!(fixture.symbol(*name), "Write");
    let VariantPatternFields::Named(named) = fields else {
        panic!("expected named field patterns");
    };
    assert_eq!(named.len(), 1);
    assert_eq!(fixture.symbol(named[0].name), "code");
    assert!(matches!(
        &ast.pattern(named[0].pattern).kind,
        PatternKind::Binding { .. }
    ));
}

#[test]
fn alternatives_and_guards_parse_inside_one_arm() {
    let (fixture, ast) = parse_clean(
        "enum Mode { On, Off } \
         func main() -> i32 { match Mode::On { Mode::On | Mode::Off if 1 < 2 => 1, _ => 0 } }",
    );
    assert_invariants(&fixture, &ast);

    let arms = match_arms(&ast, 1);
    assert_eq!(arms.len(), 2);
    let PatternKind::Or(alternatives) = &ast.pattern(arms[0].pattern).kind else {
        panic!("expected an or-pattern");
    };
    assert_eq!(alternatives.len(), 2);
    assert!(arms[0].guard.is_some());
    assert!(arms[1].guard.is_none());
    assert!(matches!(
        &ast.pattern(arms[1].pattern).kind,
        PatternKind::Wildcard
    ));
}

#[test]
fn scalar_literals_are_patterns() {
    let (fixture, ast) = parse_clean(
        "func main() -> i32 { match 1 { 0 => 0, -1 => 1, 2.5 => 2, 'x' => 3, true => 4, _ => 5 } }",
    );
    assert_invariants(&fixture, &ast);

    let arms = match_arms(&ast, 0);
    assert_eq!(arms.len(), 6);
    assert!(matches!(
        ast.pattern(arms[0].pattern).kind,
        PatternKind::Literal(PatternLiteral::Int {
            value: 0,
            negative: false,
            suffix: None,
        })
    ));
    assert!(matches!(
        ast.pattern(arms[1].pattern).kind,
        PatternKind::Literal(PatternLiteral::Int {
            value: 1,
            negative: true,
            suffix: None,
        })
    ));
    assert!(matches!(
        ast.pattern(arms[2].pattern).kind,
        PatternKind::Literal(PatternLiteral::Float {
            value,
            suffix: None,
        }) if value == 2.5
    ));
    assert!(matches!(
        ast.pattern(arms[3].pattern).kind,
        PatternKind::Literal(PatternLiteral::Char('x'))
    ));
    assert!(matches!(
        ast.pattern(arms[4].pattern).kind,
        PatternKind::Literal(PatternLiteral::Bool(true))
    ));
}

#[test]
fn a_plain_name_is_a_binding() {
    let (fixture, ast) = parse_clean("func main() -> i32 { match 1 { value => value } }");
    assert_invariants(&fixture, &ast);

    let arms = match_arms(&ast, 0);
    let PatternKind::Binding { name: symbol, .. } = &ast.pattern(arms[0].pattern).kind else {
        panic!("expected a binding pattern");
    };
    assert_eq!(fixture.symbol(*symbol), "value");
    assert!(matches!(&ast.expr(arms[0].body).kind, ExprKind::Path(_)));
}

#[test]
fn nested_variant_patterns_and_nested_alternatives_parse() {
    let (fixture, ast) = parse_clean(
        "enum Option { Some(i32), None } \
         func main() -> i32 { match Option::None { Option::Some(0 | 1) => 1, Option::None => 0 } }",
    );
    assert_invariants(&fixture, &ast);

    let arms = match_arms(&ast, 1);
    assert_eq!(arms.len(), 2);
    let PatternKind::Variant {
        fields: VariantPatternFields::Positional(patterns),
        ..
    } = &ast.pattern(arms[0].pattern).kind
    else {
        panic!("expected positional field patterns");
    };
    assert_eq!(patterns.len(), 1);
    let PatternKind::Or(alternatives) = &ast.pattern(patterns[0]).kind else {
        panic!("expected nested alternatives");
    };
    assert_eq!(alternatives.len(), 2);
}

#[test]
fn a_string_pattern_is_rejected_and_the_arm_survives() {
    let fixture = Fixture::new("func main() -> i32 { match 1 { \"text\" => 0, _ => 1 } }");
    let ast = fixture.parse();
    fixture.expect("E1011", "a string cannot be used as a pattern literal");
    assert_invariants(&fixture, &ast);
    assert_eq!(match_arms(&ast, 0).len(), 2);
}

#[test]
fn a_missing_arm_comma_is_reported_and_recovered() {
    let fixture = Fixture::new("func main() -> i32 { match 1 { 0 => 0 _ => 1 } }");
    let ast = fixture.parse();
    fixture.expect("E1002", "between match arms");
    assert_invariants(&fixture, &ast);
    assert_eq!(match_arms(&ast, 0).len(), 2);
}

#[test]
fn a_match_needs_at_least_one_arm() {
    let fixture = Fixture::new("func main() -> i32 { match 1 {} }");
    let ast = fixture.parse();
    fixture.expect("E1011", "needs at least one arm");
    assert_invariants(&fixture, &ast);
    assert!(match_arms(&ast, 0).is_empty());
}
