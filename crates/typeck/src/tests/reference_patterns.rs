//! Struct and variant patterns that match through references.

use super::support::{Fixture, check_clean};

#[test]
fn a_struct_pattern_matches_through_a_reference() {
    let (_, _, _) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func total(point: *Point) -> i32 { match point { Point { x, y } => x + y } }",
    );
    let (_, _, _) = check_clean(
        "struct Point { x: i32, y: i32 } \
         func total(point: *mut Point) -> i32 { match point { Point { x, y } => x + y } }",
    );
    let (_, _, _) = check_clean(
        "struct Point { x: i32 } \
         func total(point: **Point) -> i32 { match point { Point { x } => x } }",
    );
}

#[test]
fn a_variant_pattern_matches_through_a_reference() {
    let (_, _, _) = check_clean(
        "enum Mode { On, Off } \
         func value(mode: *Mode) -> i32 { match mode { Mode::On => 1, Mode::Off => 0 } }",
    );
    let (_, _, _) = check_clean(
        "enum Pair { Both(i32, i32), Neither } \
         func value(pair: *Pair) -> i32 { \
           match pair { Pair::Both(left, right) => left + right, Pair::Neither => 0 } \
         }",
    );
}

#[test]
fn a_nested_reference_field_is_dereferenced_by_its_pattern() {
    let (_, _, _) = check_clean(
        "struct Inner { value: i32 } \
         struct Outer { inner: *Inner } \
         func read(outer: *Outer) -> i32 { \
           match outer { Outer { inner: Inner { value } } => value } \
         }",
    );
}

#[test]
fn a_non_copy_field_cannot_be_moved_out_of_a_reference() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         struct Holder { token: Token } \
         func read(holder: *Holder) -> i32 { match holder { Holder { token } => token.value } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3018", "cannot move a `Token` value out of a reference");

    let (_, _, _) = check_clean(
        "struct Token { value: i32 } \
         struct Holder { token: Token } \
         func read(holder: *Holder) -> i32 { \
           match holder { Holder { token: Token { value } } => value } \
         }",
    );
}

#[test]
fn a_shared_reference_reads_copy_fields() {
    let (_, _, _) = check_clean(
        "struct Token { value: i32, flag: bool } \
         func read(token: *Token) -> i32 { \
           match token { Token { value, flag: true } => value, Token { value: _, flag: _ } => 0 } \
         }",
    );
}

#[test]
fn a_literal_pattern_still_needs_the_value_itself() {
    let fixture = Fixture::new("func f(value: *i32) -> i32 { match value { 0 => 0, _ => 1 } }");
    let _ = fixture.check(64);
    assert!(
        fixture.codes().iter().any(|code| code == "E3001"),
        "expected E3001, got {:?}",
        fixture.codes()
    );
}

#[test]
fn coverage_through_a_reference_reports_the_missing_variant() {
    let fixture = Fixture::new(
        "enum Mode { On, Off } func f(mode: *Mode) -> i32 { match mode { Mode::On => 1 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3032", "Mode::Off");

    let fixture = Fixture::new(
        "enum Mode { On, Off } func f(mode: *Mode) -> i32 { match mode { _ => 0, Mode::On => 1 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");
}

#[test]
fn a_recursive_enum_by_reference_does_not_expand_forever() {
    let (_, _, _) = check_clean(
        "enum List { Empty, Node(i32, *List) } \
         func sum(list: *List) -> i32 { \
           match list { List::Node(value, next) => value + sum(next), List::Empty => 0 } \
         }",
    );
    let (_, _, _) = check_clean(
        "enum List<T> { Empty, Node(T, *List<T>) } \
         func sum(list: *List<i32>) -> i32 { \
           match list { List::Node(value, next) => value + sum(next), List::Empty => 0 } \
         }",
    );
}
