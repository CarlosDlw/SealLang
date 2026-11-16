//! Sized-storage tests.

use super::support::{check_clean, check_errors};

#[test]
fn unsized_parameters_are_reported() {
    let (fixture, _, _) = check_errors("func f(text: str) {}");
    fixture.expect("E3010", "unsized");
}

#[test]
fn unsized_return_types_are_reported() {
    let (fixture, _, _) = check_errors("func f() -> str { }");
    fixture.expect("E3010", "unsized");
}

#[test]
fn unsized_bindings_are_reported() {
    let (fixture, _, _) = check_errors("func main() { let text: str = \"hi\"; }");
    fixture.expect("E3010", "unsized");
}

#[test]
fn references_to_str_are_sized_and_accepted() {
    let (fixture, _, _) = check_clean("func identity(text: *str) -> *str { text }");
    let _ = fixture;

    let (fixture, _, _) = check_clean("func main() -> *str { \"hi\" }");
    let _ = fixture;
}
