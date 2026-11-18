//! Borrow checking for trait-object reborrows and dynamic calls.

use super::support::{check_clean, check_errors};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { \
    func area(self: *Self) -> i32; \
    func set(self: *mut Self, value: i32); \
} \
impl Shape for Circle { \
    func area(self: *Circle) -> i32 { (*self).radius } \
    func set(self: *mut Circle, value: i32) { (*self).radius = value; } \
} \
func area(object: *dyn Shape) -> i32 { object.area() } \
func set(object: *mut dyn Shape, value: i32) { object.set(value); }";

#[test]
fn borrowck_allows_disjoint_dyn_calls() {
    let source = format!(
        "{SHAPE} \
         func read(first: *Circle, second: *Circle) -> i32 {{ \
             let a: *dyn Shape = first; \
             let b: *dyn Shape = second; \
             a.area() + b.area() \
         }}"
    );
    check_clean(&source);
}

#[test]
fn borrowck_rejects_an_exclusive_call_while_a_shared_loan_is_live() {
    let source = format!(
        "{SHAPE} \
         func conflict(source: *mut Circle) -> i32 {{ \
             let object: *mut dyn Shape = source; \
             let shared: *dyn Shape = &*object; \
             object.set(1); \
             area(shared) \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5009", "exclusive argument");
}

#[test]
fn borrowck_rejects_a_shared_call_while_an_exclusive_loan_is_live() {
    let source = format!(
        "{SHAPE} \
         func take(value: *mut dyn Shape) {{ }} \
         func conflict(source: *mut Circle) -> i32 {{ \
             let object: *mut dyn Shape = source; \
             let exclusive: *mut dyn Shape = &mut *object; \
             let result = object.area(); \
             take(exclusive); \
             result \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5008", "borrow");
}

#[test]
fn reborrow_registers_a_shared_loan() {
    let source = format!(
        "{SHAPE} \
         func conflict(source: *mut Circle) -> i32 {{ \
             let object: *mut dyn Shape = source; \
             let shared: *dyn Shape = &*object; \
             (*source).radius = 2; \
             area(shared) \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5001", "borrowed");
}

#[test]
fn reborrow_registers_an_exclusive_loan() {
    let source = format!(
        "{SHAPE} \
         func take(value: *mut dyn Shape) {{ }} \
         func conflict(source: *mut Circle) -> i32 {{ \
             let object: *mut dyn Shape = source; \
             let exclusive: *mut dyn Shape = &mut *object; \
             let result = (*source).radius; \
             take(exclusive); \
             result \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn reborrow_loan_conflicts_with_an_exclusive_call() {
    let source = format!(
        "{SHAPE} \
         func conflict(source: *mut Circle) -> i32 {{ \
             let object: *mut dyn Shape = source; \
             let shared: *dyn Shape = &*object; \
             object.set(1); \
             area(shared) \
         }}"
    );
    let fixture = check_errors(&source);
    assert!(
        fixture
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E5009"))
    );
}
