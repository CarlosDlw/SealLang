//! The built-in `drop(value)` in the ownership analysis.
//!
//! `drop` consumes its operand exactly like a move, so the ordinary ownership
//! rules apply afterwards. The types are aggregates because a user-defined type
//! is the only owned, non-`Copy` value the language has today.

use super::support::Fixture;

/// Runs the pipeline with `named` implementing `Drop`.
fn check_with_drop(source: &str, named: &[&str]) -> Fixture {
    let fixture = Fixture::new(source);
    fixture.check_with_destructors(named);
    fixture
}

#[test]
fn using_a_value_after_drop_is_rejected() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         func main() -> i32 {\n\
            let token = Token { value: 1 };\n\
            drop(token);\n\
            token.value\n\
         }\n",
        &["Token"],
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn dropping_the_same_value_twice_is_rejected() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         func main() {\n\
            let token = Token { value: 1 };\n\
            drop(token);\n\
            drop(token);\n\
         }\n",
        &["Token"],
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn dropping_a_field_of_a_drop_type_is_rejected() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         struct Pair { token: Token, count: i32 }\n\n\
         func main() {\n\
            let pair = Pair { token: Token { value: 1 }, count: 2 };\n\
            drop(pair.token);\n\
         }\n",
        &["Token", "Pair"],
    );
    fixture.expect(
        "E5014",
        "cannot move out of `Pair`, which implements `Drop`",
    );
}

#[test]
fn reassigning_after_drop_is_allowed() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         func main() {\n\
            let mut token = Token { value: 1 };\n\
            drop(token);\n\
            token = Token { value: 2 };\n\
         }\n",
        &["Token"],
    );
    fixture.assert_clean();
}

#[test]
fn dropping_a_copy_value_does_not_move_anything() {
    let fixture = Fixture::new("func main() { let value = 5; drop(value); let other = value; }");
    fixture.check_with_destructors(&[]);
    fixture.assert_clean();
}

#[test]
fn dropping_a_borrowed_value_is_rejected() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         func main() {\n\
            let token = Token { value: 1 };\n\
            let view = &token;\n\
            drop(token);\n\
            let keep = view;\n\
         }\n",
        &["Token"],
    );
    fixture.expect("E5002", "because it is borrowed");
}
