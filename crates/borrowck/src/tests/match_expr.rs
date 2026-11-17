//! Ownership and borrow checking of `match`.

use super::support::{Fixture, check_clean, check_errors};

fn codes(fixture: &Fixture) -> Vec<String> {
    fixture
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            diagnostic
                .code()
                .map_or_else(|| "<none>".to_owned(), |code| code.as_str().to_owned())
        })
        .collect()
}

#[test]
fn matching_moves_the_scrutinee() {
    let fixture = check_errors(
        "enum Mode { On, Off } \
         func main() -> i32 { \
           let mode = Mode::On; \
           let first = match mode { _ => 1 }; \
           let second = match mode { _ => 0 }; \
           first + second \
         }",
    );
    assert!(
        codes(&fixture).iter().any(|code| code == "E5005"),
        "expected the second match to use a moved value, got {:?}",
        codes(&fixture)
    );
}

#[test]
fn a_non_copy_field_can_be_moved_out_of_the_scrutinee() {
    check_clean(
        "struct Token { value: i32 } \
         enum Holder { Empty, Full(Token) } \
         func read(holder: Holder) -> i32 { \
           match holder { Holder::Full(token) => token.value, Holder::Empty => 0 } \
         } \
         func main() -> i32 { read(Holder::Full(Token { value: 42 })) }",
    );
}

#[test]
fn using_the_scrutinee_while_it_is_exclusively_borrowed_is_rejected() {
    let fixture = check_errors(
        "enum Mode { On, Off } \
         func toggle(mode: *mut Mode) { *mode = Mode::Off; } \
         func main() -> i32 { \
           let mut mode = Mode::On; \
           let reference = &mut mode; \
           let value = match mode { _ => 1 }; \
           toggle(reference); \
           value \
         }",
    );
    let codes = codes(&fixture);
    assert!(
        codes.iter().any(|code| code == "E5007" || code == "E5002"),
        "expected a use while borrowed, got {codes:?}"
    );
}

#[test]
fn a_match_arm_may_borrow_the_value_it_bound() {
    check_clean(
        "struct Token { value: i32 } \
         enum Holder { Empty, Full(Token) } \
         func read(token: *Token) -> i32 { (*token).value } \
         func classify(holder: Holder) -> i32 { \
           match holder { Holder::Full(token) => read(&token), Holder::Empty => 0 } \
         } \
         func main() -> i32 { classify(Holder::Full(Token { value: 42 })) }",
    );
}
