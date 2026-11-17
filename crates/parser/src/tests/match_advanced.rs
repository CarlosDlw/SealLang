//! Advanced pattern syntax: structs, ranges, `@` bindings, arrays, and blocks.

use seal_ast::{ExprKind, PatternKind, PatternLiteral};

use super::support::{Fixture, assert_invariants, body, function, parse_clean};

const STRUCT_PATTERN: &str = "\
struct Point { x: i32, y: i32 }
func pick(point: Point) -> i32 {
  match point {
    Point { x: 0, y: value @ 1..=9 } => { value },
    Point { x, y } => x + y,
  }
}
";

#[test]
fn struct_patterns_ranges_bindings_and_blocks_parse() {
    let (fixture, ast) = parse_clean(STRUCT_PATTERN);
    assert_invariants(&fixture, &ast);

    let declaration = function(&ast, 1);
    let tail = body(&ast, declaration)
        .tail
        .expect("the function has a tail expression");
    let ExprKind::Match { arms, .. } = &ast.expr(tail).kind else {
        panic!("expected a match expression");
    };
    assert_eq!(arms.len(), 2);

    let PatternKind::Struct { name, fields, .. } = &ast.pattern(arms[0].pattern).kind else {
        panic!("expected a struct pattern");
    };
    assert_eq!(fixture.symbol(*name), "Point");
    assert_eq!(fields.len(), 2);
    assert_eq!(fixture.symbol(fields[0].name), "x");
    assert!(matches!(
        &ast.pattern(fields[0].pattern).kind,
        PatternKind::Literal(PatternLiteral::Int { value: 0, .. })
    ));

    assert_eq!(fixture.symbol(fields[1].name), "y");
    let PatternKind::Binding { name, pattern } = &ast.pattern(fields[1].pattern).kind else {
        panic!("expected an @ binding");
    };
    assert_eq!(fixture.symbol(*name), "value");
    let Some(inner) = pattern else {
        panic!("the @ binding constrains a subpattern");
    };
    let PatternKind::Range {
        start,
        end,
        inclusive,
    } = &ast.pattern(*inner).kind
    else {
        panic!("expected a range pattern");
    };
    assert!(matches!(
        start,
        PatternLiteral::Int {
            value: 1,
            negative: false,
            ..
        }
    ));
    assert!(matches!(
        end,
        PatternLiteral::Int {
            value: 9,
            negative: false,
            ..
        }
    ));
    assert!(inclusive);

    assert!(matches!(&ast.expr(arms[0].body).kind, ExprKind::Block(_)));
    assert!(!matches!(&ast.expr(arms[1].body).kind, ExprKind::Block(_)));
}

#[test]
fn array_patterns_match_each_element_in_order() {
    let (fixture, ast) = parse_clean(
        "func pick(values: [i32; 2]) -> i32 { \
           match values { [first, 0] => first, [_, second] => second } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let declaration = function(&ast, 0);
    let tail = body(&ast, declaration)
        .tail
        .expect("the function has a tail expression");
    let ExprKind::Match { arms, .. } = &ast.expr(tail).kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Array {
        prefix,
        has_rest,
        suffix,
    } = &ast.pattern(arms[0].pattern).kind
    else {
        panic!("expected an array pattern");
    };
    assert!(!has_rest);
    assert!(suffix.is_empty());
    assert_eq!(prefix.len(), 2);
    assert!(matches!(
        &ast.pattern(prefix[0]).kind,
        PatternKind::Binding { .. }
    ));
    assert!(matches!(
        &ast.pattern(prefix[1]).kind,
        PatternKind::Literal(PatternLiteral::Int { value: 0, .. })
    ));

    let PatternKind::Array { prefix, .. } = &ast.pattern(arms[1].pattern).kind else {
        panic!("expected an array pattern");
    };
    assert!(matches!(
        &ast.pattern(prefix[0]).kind,
        PatternKind::Wildcard
    ));
}

#[test]
fn a_rest_pattern_skips_the_elements_in_between() {
    let (fixture, ast) = parse_clean(
        "func main(values: [i32; 3]) -> i32 { \
           match values { [first, .., last] => first + last } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let declaration = function(&ast, 0);
    let tail = body(&ast, declaration)
        .tail
        .expect("the function has a tail expression");
    let ExprKind::Match { arms, .. } = &ast.expr(tail).kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Array {
        prefix,
        has_rest,
        suffix,
    } = &ast.pattern(arms[0].pattern).kind
    else {
        panic!("expected an array pattern");
    };
    assert!(has_rest);
    assert_eq!(prefix.len(), 1);
    assert_eq!(suffix.len(), 1);
}

#[test]
fn a_range_without_an_end_is_reported() {
    let fixture = Fixture::new("func main(value: i32) -> i32 { match value { 0.. => 0, _ => 1 } }");
    let ast = fixture.parse();
    fixture.expect("E1011", "range end");
    assert_invariants(&fixture, &ast);
}
