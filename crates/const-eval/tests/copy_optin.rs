//! Constant evaluation of opt-in `Copy` values.
//!
//! A `const` of a `Copy` ADT may be read by value, and copying it must not
//! consume the original constant.

mod support;

use seal_const_eval::ConstValue;
use support::{Fixture, clean};

const POINT: &str = "struct Point { x: i32, y: i32 } impl Copy for Point {}";

fn int(fixture: &Fixture, name: &str) -> u128 {
    match fixture.value(name) {
        ConstValue::Int { bits, .. } => *bits,
        other => panic!("`{name}` is not an integer: {other:?}"),
    }
}

#[test]
fn a_copy_constant_is_readable_without_being_consumed() {
    let source = format!(
        "{POINT} \
         const ORIGIN: Point = Point {{ x: 41, y: 1 }}; \
         const COPY: Point = ORIGIN; \
         const X: i32 = ORIGIN.x; \
         const Y: i32 = COPY.y;"
    );
    let fixture = clean(&source);
    assert_eq!(int(&fixture, "X"), 41);
    assert_eq!(int(&fixture, "Y"), 1);
    assert!(matches!(
        fixture.value("ORIGIN"),
        ConstValue::Struct { fields, .. } if fields.len() == 2
    ));
    assert!(matches!(
        fixture.value("COPY"),
        ConstValue::Struct { fields, .. } if fields.len() == 2
    ));
}

#[test]
fn a_copy_static_initializes_and_is_readable_by_value() {
    let fixture = clean(&format!(
        "{POINT} \
         static HOME: Point = Point {{ x: 20, y: 22 }}; \
         const X: i32 = HOME.x; \
         const Y: i32 = HOME.y;"
    ));
    assert_eq!(int(&fixture, "X"), 20);
    assert_eq!(int(&fixture, "Y"), 22);
    assert!(matches!(
        fixture.value("HOME"),
        ConstValue::Struct { fields, .. } if fields.len() == 2
    ));
}

#[test]
fn a_copy_const_function_result_is_reusable() {
    let fixture = clean(&format!(
        "{POINT} \
         const func make() -> Point {{ Point {{ x: 40, y: 2 }} }} \
         const FIRST: Point = make(); \
         const SECOND: Point = make(); \
         const X: i32 = FIRST.x + SECOND.y;"
    ));
    assert_eq!(int(&fixture, "X"), 42);
}
