//! Moves and initialization around types that own a value.
//!
//! Three rules protect a value whose declared type runs a destructor: a field
//! may not leave it (`E5014`), an element may not leave an array of non-`Copy`
//! values (`E5015`), and a field may not be the only thing that ever initializes
//! such a value (`E5016`). `impl Drop` is not part of the surface yet, so each
//! test registers a destructor for the aggregate before MIR lowering.
//!
//! The `E5014` case also covers moving the whole value, which stays legal.

use super::support::Fixture;

/// Runs the pipeline with `named` implementing `Drop`.
fn check_with_drop(source: &str, named: &[&str]) -> Fixture {
    let fixture = Fixture::new(source);
    fixture.check_with_destructors(named);
    fixture
}

#[test]
fn moving_a_field_out_of_a_drop_type_is_rejected() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         struct Pair { token: Token, count: i32 }\n\n\
         func steal(pair: Pair) -> Token { pair.token }\n",
        &["Pair"],
    );
    fixture.expect(
        "E5014",
        "cannot move out of `Pair`, which implements `Drop`",
    );
}

#[test]
fn moving_a_whole_drop_value_is_allowed() {
    let fixture = check_with_drop(
        "struct Token { value: i32 }\n\n\
         struct Pair { token: Token, count: i32 }\n\n\
         func pass(pair: Pair) -> Pair { pair }\n\n\
         func main() -> i32 { 0 }\n",
        &["Pair"],
    );
    fixture.assert_clean();
}

#[test]
fn moving_an_element_out_of_an_array_is_rejected() {
    let fixture = check_with_drop(
        "struct Pair { first: i32, second: i32 }\n\n\
         func first(values: [Pair; 2]) -> Pair { values[0] }\n",
        &[],
    );
    fixture.expect("E5015", "cannot move an element out of an array");
}

#[test]
fn copying_an_element_out_of_an_array_is_allowed() {
    let fixture = check_with_drop("func first(values: [i32; 2]) -> i32 { values[0] }\n", &[]);
    fixture.assert_clean();
}

#[test]
fn initializing_one_field_of_a_drop_value_is_rejected() {
    let fixture = check_with_drop(
        "struct Item { value: i32 }\n\n\
         func build(value: i32) {\n  let mut item: Item;\n  item.value = value;\n}\n",
        &["Item"],
    );
    fixture.expect(
        "E5016",
        "cannot initialize one field of a value that needs `Drop`",
    );
}

#[test]
fn assigning_a_field_of_an_initialized_value_is_allowed() {
    let fixture = check_with_drop(
        "struct Item { value: i32 }\n\n\
         func build(value: i32) -> Item {\n  \
         let mut item = Item { value: 0 };\n  item.value = value;\n  item\n}\n",
        &["Item"],
    );
    fixture.assert_clean();
}
