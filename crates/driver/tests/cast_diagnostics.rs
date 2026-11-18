//! Command-line regression fixtures for explicit cast diagnostics.
//!
//! Each case pins the stable code, the primary source location, the secondary
//! labels around the operand and target, and exit status 1.

mod support;

use support::{SourceFile, reject};

/// Requires `source` to be rejected at `location` with `code`.
fn reject_at(name: &str, source: &str, code: &str, location: &str, fragments: &[&str]) {
    let file = SourceFile::new(name, source);
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{name}: expected exit status 1, got {:?}: {stderr}",
        output.status
    );
    assert!(
        stderr.contains(&format!("error[{code}]")),
        "{name}: expected {code}, got: {stderr}"
    );
    assert!(
        stderr.contains(&format!(":{location}")),
        "{name}: expected the primary location {location}, got: {stderr}"
    );
    for fragment in fragments {
        assert!(
            stderr.contains(fragment),
            "{name}: expected {fragment:?} in: {stderr}"
        );
    }
}

#[test]
fn a_numeric_value_cannot_be_cast_to_bool() {
    reject_at(
        "cast-numeric-bool",
        "func f(value: i32) -> bool { value as bool }\nfunc main() -> i32 { 0 }\n",
        "E3048",
        "1:36",
        &[
            "cannot cast `i32` to `bool`",
            "operand has type `i32`",
            "target type is `bool`",
            "`bool.to_i32()`",
        ],
    );
}

#[test]
fn a_char_cannot_be_cast_to_a_number() {
    reject_at(
        "cast-char-numeric",
        "func f(value: char) -> u32 { value as u32 }\nfunc main() -> i32 { 0 }\n",
        "E3048",
        "1:36",
        &[
            "cannot cast `char` to `u32`",
            "operand has type `char`",
            "`char.to_u32()`",
        ],
    );
}

#[test]
fn a_reference_cannot_be_cast_to_a_number() {
    reject_at(
        "cast-reference-numeric",
        "func f(value: *i32) -> i32 { value as i32 }\nfunc main() -> i32 { 0 }\n",
        "E3048",
        "1:36",
        &["cannot cast `*i32` to `i32`", "operand has type `*i32`"],
    );
}

#[test]
fn an_aggregate_cannot_be_cast_to_a_number() {
    reject_at(
        "cast-aggregate-numeric",
        "struct Point { x: i32 }\nfunc f(value: Point) -> i32 { value as i32 }\nfunc main() -> i32 { 0 }\n",
        "E3048",
        "2:37",
        &["cannot cast `Point` to `i32`", "operand has type `Point`"],
    );
}

#[test]
fn a_type_parameter_source_points_at_the_operand() {
    reject_at(
        "cast-parameter-source",
        "func f<T>(value: T) -> i32 { value as i32 }\nfunc main() -> i32 { 0 }\n",
        "E3049",
        "1:30",
        &[
            "cannot cast type parameter `T` to `i32`",
            "the cast must name a concrete numeric conversion",
            "no numeric-bound trait",
        ],
    );
}

#[test]
fn a_type_parameter_target_points_at_the_target() {
    reject_at(
        "cast-parameter-target",
        "func f<T>(value: i32) -> T { value as T }\nfunc main() -> i32 { 0 }\n",
        "E3049",
        "1:39",
        &[
            "cannot cast `i32` to type parameter `T`",
            "the target type is generic, not a numeric type",
            "no numeric-bound trait",
        ],
    );
}

#[test]
fn a_missing_target_type_is_a_parser_error() {
    let output = reject(
        "cast-missing-target",
        "func f(value: i32) -> i32 { value as; 0 }\nfunc main() -> i32 { 0 }\n",
        "E1004",
        "expected a type, found `;`",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(":1:37"),
        "expected the parser error to point at the missing type: {stderr}"
    );
}
