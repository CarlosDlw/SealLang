//! Type checking of values whose type opts into `Copy`.
//!
//! A `Copy` type may be used by value without moving the source, so every
//! expression that consumes a value must accept both the consumed value and a
//! later use of the original place.

use super::support::{check_clean, check_errors};

const POINT: &str = "struct Point { x: i32, y: i32 } impl Copy for Point {}";
const TOKEN: &str = "struct Token { value: i32 }";

#[test]
fn copying_a_struct_leaves_both_bindings_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let a = Point {{ x: 1, y: 2 }}; \
           let b = a; \
           a.x + b.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn passing_a_copy_value_leaves_the_caller_usable() {
    let source = format!(
        "{POINT} \
         func take(point: Point) -> i32 {{ point.x }} \
         func main() -> i32 {{ \
           let point = Point {{ x: 20, y: 22 }}; \
           take(point) + point.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn returning_a_copy_value_leaves_the_argument_usable() {
    let source = format!(
        "{POINT} \
         func pass(point: Point) -> Point {{ point }} \
         func main() -> i32 {{ \
           let point = Point {{ x: 40, y: 2 }}; \
           let result = pass(point); \
           point.x + result.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn reading_a_copy_field_leaves_the_struct_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let point = Point {{ x: 40, y: 2 }}; \
           let x = point.x; \
           x + point.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn a_match_binding_of_a_copy_value_leaves_the_scrutinee_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let point = Point {{ x: 40, y: 2 }}; \
           let value = match point {{ Point {{ x, y }} => x + y }}; \
           point.x + value \
         }}"
    );
    check_clean(&source);
}

#[test]
fn reading_a_copy_array_element_leaves_the_array_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let points = [Point {{ x: 20, y: 1 }}, Point {{ x: 1, y: 20 }}]; \
           let first = points[0]; \
           first.x + points[1].y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn iterating_a_copy_array_by_value_leaves_the_source_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let points = [Point {{ x: 20, y: 0 }}, Point {{ x: 1, y: 0 }}]; \
           let mut total = 0; \
           for point in points {{ total += point.x; }} \
           total + points[1].x \
         }}"
    );
    check_clean(&source);
}

#[test]
fn repeating_a_copy_struct_array_is_allowed() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let points = [Point {{ x: 3, y: 4 }}; 3]; \
           points[2].x \
         }}"
    );
    check_clean(&source);
}

#[test]
fn dropping_a_copy_value_leaves_it_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let point = Point {{ x: 42, y: 0 }}; \
           drop(point); \
           point.x \
         }}"
    );
    check_clean(&source);
}

#[test]
fn reading_and_dropping_a_copy_static_is_allowed() {
    let source = format!(
        "{POINT} \
         static HOME: Point = Point {{ x: 40, y: 2 }}; \
         func main() -> i32 {{ \
           drop(HOME); \
           HOME.x + HOME.y \
         }}"
    );
    check_clean(&source);
}

#[test]
fn a_copy_bound_accepts_a_copy_adt_and_rejects_a_non_copy_one() {
    let source = format!(
        "{POINT} {TOKEN} \
         func pass<T: Copy>(value: T) -> T {{ value }} \
         func good() {{ pass(Point {{ x: 1, y: 2 }}); }} \
         func bad() {{ pass(Token {{ value: 1 }}); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3043", "does not satisfy the `Copy` bound");
}

#[test]
fn dropping_a_non_copy_static_is_still_rejected() {
    let source = format!(
        "{TOKEN} \
         static TOKEN: Token = Token {{ value: 1 }}; \
         func main() {{ drop(TOKEN); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3058", "out of a static");
}

#[test]
fn reading_a_copy_static_mut_by_value_is_allowed_in_unsafe() {
    let source = format!(
        "{POINT} \
         static mut HOME: Point = Point {{ x: 40, y: 2 }}; \
         func main() -> i32 {{ \
           unsafe {{ \
             let home = HOME; \
             home.x + home.y \
           }} \
         }}"
    );
    check_clean(&source);
}

#[test]
fn equality_of_copy_structs_leaves_both_operands_usable() {
    let source = format!(
        "{POINT} \
         func main() -> i32 {{ \
           let left = Point {{ x: 40, y: 2 }}; \
           let right = Point {{ x: 40, y: 2 }}; \
           if left == right {{ left.x + right.y }} else {{ 0 }} \
         }}"
    );
    check_clean(&source);
}

#[test]
fn if_let_and_while_let_keep_a_copy_scrutinee_usable() {
    let source = format!(
        "enum Maybe {{ Some(Point), None }} \
         impl Copy for Maybe {{}} \
         {POINT} \
         func main() -> i32 {{ \
           let maybe = Maybe::Some(Point {{ x: 40, y: 2 }}); \
           let first = if let Maybe::Some(point) = maybe {{ point.x }} else {{ 0 }}; \
           let second = if let Maybe::Some(point) = maybe {{ point.y }} else {{ 0 }}; \
           let mut remaining = maybe; \
           let mut total = 0; \
           while let Maybe::Some(point) = remaining {{ \
             total += point.x + point.y; \
             remaining = Maybe::None; \
           }} \
           first + second + total \
         }}"
    );
    check_clean(&source);
}

#[test]
fn a_conditional_copy_impl_requires_copy_arguments() {
    let source = format!(
        "struct Wrapper<T> {{ value: T }} \
         impl<T: Copy> Copy for Wrapper<T> {{}} \
         {TOKEN} \
         func pass<T: Copy>(value: T) {{ }} \
         func good() {{ pass(Wrapper {{ value: 1 }}); }} \
         func bad() {{ pass(Wrapper {{ value: Token {{ value: 1 }} }}); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3043", "does not satisfy the `Copy` bound");
}

#[test]
fn a_conditional_wrapper_is_copy_inside_a_generic_body() {
    let source = "\
        struct Wrapper<T> { value: T } \
        impl<T: Copy> Copy for Wrapper<T> {} \
        func duplicate<T: Copy>(value: Wrapper<T>) -> (Wrapper<T>, Wrapper<T>) { \
          (value, value) \
        }";
    check_clean(source);
}

#[test]
fn copy_cannot_be_used_as_a_type() {
    let (fixture, _, _) = check_errors("func take(value: Copy) {}");
    fixture.expect("E2027", "is a trait, not a type");
}

#[test]
fn copy_has_no_associated_items_to_call() {
    let (fixture, _, _) = check_errors("func main() { Copy::algo(); }");
    fixture.expect("E2018", "Copy");
}

#[test]
fn a_generic_match_guard_can_read_a_copy_binding() {
    let source = "\
        struct Wrapper<T> { value: T } \
        impl<T: Copy> Copy for Wrapper<T> {} \
        func always<T>(value: T) -> bool { true } \
        func pick<T: Copy>(wrapper: Wrapper<T>) -> T { \
          match wrapper { \
            Wrapper { value } if always(value) => value, \
            Wrapper { value } => value, \
          } \
        }";
    check_clean(source);
}
