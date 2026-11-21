//! Validation of functions exported to C.

use super::support::Fixture;

#[test]
fn a_concrete_export_with_c_types_is_accepted() {
    let fixture = Fixture::new(
        r#"
export "c" func add(a: i32, b: i32) -> i32 {
  a + b
}
"#,
    );
    fixture.check(64);
    fixture.assert_clean();
}

#[test]
fn a_generic_export_is_rejected() {
    let fixture = Fixture::new(
        r#"
export "c" func identity<T>(value: T) -> T {
  value
}
"#,
    );
    fixture.check(64);
    fixture.expect("E3084", "cannot be generic");
}

#[test]
fn the_c_entry_point_name_cannot_be_exported() {
    let fixture = Fixture::new(
        r#"
export "c" func main() -> i32 {
  0
}
"#,
    );
    fixture.check(64);
    fixture.expect("E3082", "reserved for the C entry point");
}

#[test]
fn two_exports_cannot_claim_one_c_name() {
    let fixture = Fixture::new(
        r#"
export "c" func add(a: i32) -> i32 {
  a
}

export "c" func add(b: i32) -> i32 {
  b
}
"#,
    );
    fixture.check(64);
    fixture.expect("E3082", "exported more than once");
}

#[test]
fn a_non_ffi_safe_export_type_is_rejected() {
    let fixture = Fixture::new(
        r#"
export "c" func flag(value: i128) -> i32 {
  0
}
"#,
    );
    fixture.check(64);
    fixture.expect("E3084", "is not FFI-safe");
}
