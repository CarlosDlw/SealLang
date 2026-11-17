//! Parsing `for` loops.

use seal_ast::{ForSource, PatternKind, StmtKind};

use super::support::{Fixture, assert_invariants, body, function, parse_clean};

#[test]
fn for_loops_parse_sequences_patterns_and_both_range_forms() {
    let (fixture, ast) = parse_clean(
        "func main(values: [i32; 3]) -> i32 { \
           let mut total = 0; \
           for value in values { total += value; } \
           for index in 0..=2 { total += index; } \
           total \
         }",
    );
    assert_invariants(&fixture, &ast);

    let declaration = function(&ast, 0);
    let statements = &body(&ast, declaration).statements;
    assert_eq!(statements.len(), 3);

    let StmtKind::For {
        pattern,
        source: ForSource::Expr(_),
        body: loop_body,
    } = &ast.stmt(statements[1]).kind
    else {
        panic!("expected a for loop over a sequence");
    };
    let PatternKind::Binding { name, .. } = &ast.pattern(*pattern).kind else {
        panic!("expected a binding pattern");
    };
    assert_eq!(fixture.symbol(*name), "value");
    assert_eq!(ast.block(*loop_body).statements.len(), 1);

    let StmtKind::For {
        pattern,
        source: ForSource::Range { inclusive, .. },
        ..
    } = &ast.stmt(statements[2]).kind
    else {
        panic!("expected a for loop over a range");
    };
    assert!(*inclusive);
    assert!(matches!(
        &ast.pattern(*pattern).kind,
        PatternKind::Binding { .. }
    ));
}

#[test]
fn a_missing_in_after_the_for_pattern_is_reported() {
    let fixture = Fixture::new("func main() -> i32 { for value 0..3 { } 0 }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`in`");
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_range_without_an_end_in_a_for_loop_is_reported() {
    let fixture = Fixture::new("func main() -> i32 { for value in 0.. { } 0 }");
    let ast = fixture.parse();
    fixture.expect("E1011", "end expression");
    assert_invariants(&fixture, &ast);
}
