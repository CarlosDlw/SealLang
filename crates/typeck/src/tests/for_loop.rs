//! Type checking `for` loops.

use super::support::{Fixture, check_clean};

#[test]
fn for_loops_accept_ranges_arrays_references_and_irrefutable_patterns() {
    let (_, _, _) = check_clean(
        "func main(values: [i32; 4]) -> i32 { \
           let mut total = 0; \
           for value in values { total += value; } \
           for value in &values { total += *value; } \
           total \
         }\n\
         func indexed(values: *[i32]) -> i32 { \
           let mut total = 0; \
           for index in 0..len(values) { total += values[index]; } \
           total \
         }",
    );
    let (_, _, _) = check_clean(
        "struct Point { x: i32 } \
         func main(points: [Point; 1]) -> i32 { \
           let mut total = 0; \
           for Point { x } in points { total += x; } \
           total \
         }",
    );
    let (_, _, _) = check_clean(
        "func main(values: *mut [i32]) -> i32 { \
           let mut total = 0; \
           for value in values { *value += 1; total += *value; } \
           total \
         }",
    );
}

#[test]
fn a_refutable_pattern_is_rejected() {
    let fixture = Fixture::new("func main() -> i32 { for 0 in 0..3 { } 0 }");
    let _ = fixture.check(64);
    fixture.expect("E3034", "does not match every element");
}

#[test]
fn a_non_iterable_source_is_rejected() {
    let fixture = Fixture::new("func main() -> i32 { for value in 5 { } 0 }");
    let _ = fixture.check(64);
    fixture.expect("E3035", "cannot iterate over");

    let fixture = Fixture::new(
        "func value() -> f64 { 1.0 } func main() -> i32 { for v in 0.0..value() { } 0 }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3035", "integer bounds");
}
