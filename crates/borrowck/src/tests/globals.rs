//! Loans rooted at static and const allocations.

use super::support::{check_clean, check_errors};

#[test]
fn a_call_that_writes_a_borrowed_static_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func reset() {\n  unsafe { COUNTER = 0; }\n}\n\
         func broken() -> i32 {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    reset();\n    *value\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn a_call_that_reads_an_exclusively_borrowed_static_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func peek() -> i32 {\n  unsafe { COUNTER }\n}\n\
         func broken() -> i32 {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    let observed = peek();\n    *value + observed\n  }\n\
         }\n",
    );
    fixture.expect("E5011", "may read the mutable static `COUNTER`");
}

#[test]
fn effects_are_transitive_through_calls() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func inner() {\n  unsafe { COUNTER = 1; }\n}\n\
         func outer() {\n  inner();\n}\n\
         func broken() {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    outer();\n    *value = 2;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn recursive_effects_reach_a_fixed_point() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func countdown(value: i32) {\n  \
           unsafe {\n    if value > 0 {\n      COUNTER = value;\n      countdown(value - 1);\n    }\n  }\n\
         }\n\
         func broken() {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    countdown(2);\n    *value = 0;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn a_reference_argument_does_not_duplicate_its_own_effect() {
    check_clean(
        "static mut COUNTER: i32 = 0;\n\
         func read(value: *mut i32) -> i32 {\n  \
           unsafe { *value + COUNTER }\n\
         }\n\
         func use_counter() -> i32 {\n  \
           unsafe { read(&mut COUNTER) }\n\
         }\n",
    );
}

#[test]
fn passing_an_exclusive_static_reference_affects_callers() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func touch(value: *mut i32) {\n  unsafe { *value = 1; }\n}\n\
         func use_counter() {\n  unsafe { touch(&mut COUNTER); }\n}\n\
         func broken() {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    use_counter();\n    *value = 2;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn disjoint_static_fields_do_not_conflict_across_calls() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write_left() {\n  unsafe { PAIR.left = 1; }\n}\n\
         func use_right() {\n  \
           unsafe {\n    let right = &mut PAIR.right;\n    write_left();\n    *right = 2;\n  }\n\
         }\n",
    );
}

#[test]
fn a_field_argument_does_not_hide_a_whole_static_write() {
    let fixture = check_errors(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write_whole() {\n  unsafe { PAIR = Pair { left: 1, right: 2 }; }\n}\n\
         func broken() {\n  \
           unsafe {\n    let right = &mut PAIR.right;\n    write_whole();\n    *right = 3;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `PAIR`");
}

#[test]
fn a_runtime_index_covers_the_whole_array() {
    let fixture = check_errors(
        "static mut TABLE: [i32; 2] = [0, 0];\n\
         func write(index: usize) {\n  unsafe { TABLE[index] = 1; }\n}\n\
         func broken() {\n  \
           unsafe {\n    let element = &mut TABLE[1];\n    write(0);\n    *element = 2;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `TABLE`");
}

#[test]
fn constant_indices_stay_disjoint_across_calls() {
    check_clean(
        "static mut TABLE: [i32; 2] = [0, 0];\n\
         func write_first() {\n  unsafe { TABLE[0] = 1; }\n}\n\
         func use_second() {\n  \
           unsafe {\n    let second = &mut TABLE[1];\n    write_first();\n    *second = 2;\n  }\n\
         }\n",
    );
}

#[test]
fn trait_bound_calls_union_every_implementation() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         struct Item {}\n\
         trait Touch { func touch(self: *Self); }\n\
         impl Touch for Item {\n  func touch(self: *Item) { unsafe { COUNTER = 1; } }\n}\n\
         func run<T: Touch>(value: *T) { value.touch(); }\n\
         func broken() {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    let item = Item {};\n    run(&item);\n    *value = 2;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn dynamic_calls_union_every_implementation() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         struct Item {}\n\
         trait Touch { func touch(self: *mut Self); }\n\
         impl Touch for Item {\n  func touch(self: *mut Item) { unsafe { COUNTER = 1; } }\n}\n\
         func run(value: *mut dyn Touch) { value.touch(); }\n\
         func broken(item: *mut Item) {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    let object: *mut dyn Touch = item;\n    run(object);\n    *value = 2;\n  }\n\
         }\n",
    );
    fixture.expect("E5013", "may write the mutable static `COUNTER`");
}

#[test]
fn borrowing_a_mutable_static_inside_unsafe_is_allowed() {
    check_clean(
        "static mut COUNTER: i32 = 0;\n\
         func bump() -> i32 {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    *value = *value + 1;\n    *value\n  }\n\
         }\n",
    );
}

#[test]
fn disjoint_fields_of_a_mutable_static_do_not_conflict() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write() {\n  \
           unsafe {\n    let left = &mut PAIR.left;\n    let right = &mut PAIR.right;\n    *left = 1;\n    *right = 2;\n  }\n\
         }\n",
    );
}

#[test]
fn a_whole_mutable_static_conflicts_with_a_field_loan() {
    let fixture = check_errors(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write() {\n  \
           unsafe {\n    let field = &mut PAIR.left;\n    let whole = &mut PAIR;\n    *field = 1;\n  }\n\
         }\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn returning_a_mutable_static_reference_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func leak() -> *mut i32 {\n  unsafe { &mut COUNTER }\n}\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
    assert!(
        !fixture
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E5004")),
        "a mutable static must not also be reported as a dangling local"
    );
}

#[test]
fn returning_a_mutable_static_reference_through_a_call_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func identity(value: *mut i32) -> *mut i32 {\n  value\n}\n\
         func leak() -> *mut i32 {\n  unsafe { identity(&mut COUNTER) }\n}\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
}

#[test]
fn returning_a_mutable_static_reference_through_a_field_is_rejected() {
    let fixture = check_errors(
        "struct Holder { value: *mut i32 }\n\
         static mut COUNTER: i32 = 0;\n\
         func leak() -> *mut i32 {\n  \
           unsafe {\n    let holder = Holder { value: &mut COUNTER };\n    holder.value\n  }\n\
         }\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
}

#[test]
fn returning_the_address_of_a_const_field_is_allowed() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         const PAIR: Pair = Pair { left: 1, right: 2 };\n\
         func address() -> *i32 {\n  &PAIR.left\n}\n",
    );
}

#[test]
fn returning_an_addressed_const_is_allowed() {
    check_clean(
        "const VALUE: i32 = 1;\n\
         func address() -> *i32 {\n  &VALUE\n}\n",
    );
}

#[test]
fn returning_a_local_copy_of_a_const_is_rejected() {
    let fixture = check_errors(
        "const VALUE: i32 = 1;\n\
         func address() -> *i32 {\n  let local = VALUE;\n  &local\n}\n",
    );
    fixture.expect("E5004", "local of this function");
}

#[test]
fn returning_a_shared_static_reference_is_allowed() {
    check_clean(
        "static LIMIT: i32 = 10;\n\
         func address() -> *i32 {\n  &LIMIT\n}\n",
    );
}

#[test]
fn returning_an_exclusive_static_reference_without_returning_it_is_allowed() {
    check_clean(
        "static mut COUNTER: i32 = 0;\n\
         func identity(value: *mut i32) -> *mut i32 {\n  value\n}\n\
         func use_counter() -> i32 {\n  \
           unsafe {\n    let value = identity(&mut COUNTER);\n    *value = 1;\n    *value\n  }\n\
         }\n",
    );
}

#[test]
fn returning_a_const_reference_names_its_static_allocation() {
    check_clean(
        "static BASE: i32 = 5;\n\
         const REF: *i32 = &BASE;\n\
         func address() -> *i32 {\n  REF\n}\n",
    );
}

#[test]
fn returning_a_const_reference_through_a_field_is_allowed() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         static PAIR: Pair = Pair { left: 1, right: 2 };\n\
         const LEFT: *i32 = &PAIR.left;\n\
         func address() -> *i32 {\n  LEFT\n}\n",
    );
}
