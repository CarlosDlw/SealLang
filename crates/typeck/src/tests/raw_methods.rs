//! Raw pointer intrinsic method signatures and the `unsafe` gate.

use crate::{ReceiverAdjust, ReceiverForm};

use super::support::{check_clean, check_errors, expr_ty, function, tail, ty_text};

#[test]
fn safe_pointer_queries_need_no_unsafe_block() {
    check_clean(
        "func f(p: *raw i32) -> bool { p.is_null() } \
         func g(p: *raw i32) -> usize { p.addr() } \
         func h(p: *raw i32) -> usize { p.expose_addr() } \
         func i(p: *raw i32) -> *raw i32 { p.with_addr(0usize) } \
         func j(p: *raw i32) -> *raw i32 { p.wrapping_add(1).wrapping_sub(1).wrapping_offset(-1isize) }",
    );
}

#[test]
fn unsafe_pointer_operations_require_a_block() {
    for source in [
        "func f(p: *raw i32) -> *raw i32 { p.add(1) }",
        "func f(p: *raw i32) -> *raw i32 { p.sub(1) }",
        "func f(p: *raw i32) -> *raw i32 { p.offset(1isize) }",
        "func f(p: *raw i32) -> *raw i32 { p.byte_add(1) }",
        "func f(p: *raw i32) -> *raw i32 { p.byte_offset(1isize) }",
        "func f(p: *raw i32) -> isize { p.offset_from(p) }",
        "func f(p: *raw i32) -> i32 { p.read() }",
        "func f(p: *raw mut i32, v: i32) { p.write(v); }",
        "func f(p: *raw i32) -> i32 { p.read_unaligned() }",
        "func f(p: *raw mut i32, v: i32) { p.write_unaligned(v); }",
        "func f(p: *raw mut i32, q: *raw i32, n: usize) { p.copy(q, n); }",
        "func f(p: *raw mut i32, q: *raw i32, n: usize) { p.copy_nonoverlapping(q, n); }",
        "func f(p: *raw u8) -> *raw [u8] { p.with_len(1) }",
        "func f(p: *raw [u8]) -> *raw str { p.as_str() }",
    ] {
        let (fixture, _, _) = check_errors(source);
        fixture.expect("E3070", "requires an `unsafe` block");
    }
}

#[test]
fn pointer_operations_produce_the_documented_types() {
    let (fixture, hir, results) = check_clean(
        "func add(p: *raw i32) -> *raw i32 { unsafe { p.add(1) } } \
         func read(p: *raw i32) -> i32 { unsafe { p.read() } } \
         func distance(p: *raw i32, q: *raw i32) -> isize { unsafe { p.offset_from(q) } }",
    );
    for (name, expected) in [("add", "*raw i32"), ("read", "i32"), ("distance", "isize")] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}

#[test]
fn a_pointer_receiver_is_read_by_value() {
    let (fixture, hir, results) = check_clean("func f(p: *raw i32) -> bool { p.is_null() }");
    let (_, body) = function(&fixture, &hir, "f");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("the method resolves");
    assert_eq!(resolved.receiver_form, ReceiverForm::Value);
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::None);
}

#[test]
fn data_discards_the_metadata() {
    let (fixture, hir, results) = check_clean(
        "func slice(p: *raw [i32]) -> *raw i32 { p.data() } \
         func text(p: *raw str) -> *raw u8 { p.data() }",
    );
    for (name, expected) in [("slice", "*raw i32"), ("text", "*raw u8")] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}

#[test]
fn length_reads_fat_pointer_metadata() {
    let (fixture, hir, results) = check_clean(
        "func slice(p: *raw [i32]) -> usize { p.len() } \
         func text(p: *raw str) -> usize { p.len() }",
    );
    for name in ["slice", "text"] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), "usize");
    }
}

#[test]
fn with_len_builds_a_slice_pointer_and_preserves_kind() {
    let (fixture, hir, results) = check_clean(
        "func shared(p: *raw u8) -> *raw [u8] { unsafe { p.with_len(5) } } \
         func mutable(p: *raw mut u8) -> *raw mut [u8] { unsafe { p.with_len(5) } }",
    );
    for (name, expected) in [("shared", "*raw [u8]"), ("mutable", "*raw mut [u8]")] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}

#[test]
fn as_str_reinterprets_a_byte_slice_pointer() {
    let (fixture, hir, results) = check_clean(
        "func shared(p: *raw [u8]) -> *raw str { unsafe { p.as_str() } } \
         func mutable(p: *raw mut [u8]) -> *raw mut str { unsafe { p.as_str() } }",
    );
    for (name, expected) in [("shared", "*raw str"), ("mutable", "*raw mut str")] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}

#[test]
fn with_len_rejects_a_fat_pointer() {
    let (fixture, _, _) =
        check_errors("func f(p: *raw [u8]) -> *raw [u8] { unsafe { p.with_len(1) } }");
    fixture.expect("E3075", "requires a thin pointer to a sized pointee");
}

#[test]
fn as_str_rejects_a_non_byte_slice() {
    let (fixture, _, _) =
        check_errors("func f(p: *raw [i32]) -> *raw str { unsafe { p.as_str() } }");
    fixture.expect("E3075", "requires a raw pointer to `[u8]`");
}

#[test]
fn length_rejects_a_thin_pointer() {
    let (fixture, _, _) = check_errors("func f(p: *raw i32) -> usize { p.len() }");
    fixture.expect("E3075", "requires a raw pointer to a slice or `str`");
}

#[test]
fn arithmetic_rejects_a_fat_pointer_until_data_is_called() {
    let (fixture, _, _) =
        check_errors("func f(p: *raw [i32]) -> *raw [i32] { unsafe { p.add(1) } }");
    fixture.expect("E3075", "requires a sized pointee");

    check_clean("func f(p: *raw [i32]) -> *raw i32 { unsafe { p.data().add(1) } }");
}

#[test]
fn writes_require_a_mutable_raw_pointer() {
    for source in [
        "func f(p: *raw i32, v: i32) { unsafe { p.write(v); } }",
        "func f(p: *raw i32, v: i32) { unsafe { p.write_unaligned(v); } }",
        "func f(p: *raw i32, q: *raw i32, n: usize) { unsafe { p.copy(q, n); } }",
        "func f(p: *raw i32, q: *raw i32, n: usize) { unsafe { p.copy_nonoverlapping(q, n); } }",
    ] {
        let (fixture, _, _) = check_errors(source);
        fixture.expect("E3075", "writes through a `*raw T` pointer");
    }

    check_clean(
        "func f(p: *raw mut i32, q: *raw i32, n: usize, v: i32) { unsafe { p.write(v); p.write_unaligned(v); p.copy(q, n); p.copy_nonoverlapping(q, n); } }",
    );
}

#[test]
fn write_checks_the_pointee_type() {
    let (fixture, _, _) =
        check_errors("func f(p: *raw mut i32, value: bool) { unsafe { p.write(value); } }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_zero_sized_pointee_is_sized_for_arithmetic() {
    check_clean("func f(p: *raw ()) -> *raw () { unsafe { p.add(1) } }");
}

#[test]
fn offset_from_keeps_the_receiver_pointer_kind() {
    check_clean(
        "func f(p: *raw mut i32, q: *raw mut i32) -> isize { unsafe { p.offset_from(q) } }",
    );
}

#[test]
fn data_discards_dyn_metadata() {
    let (fixture, hir, results) = check_clean(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
         func object(pointer: *raw dyn Shape) -> *raw () { pointer.data() }",
    );
    let (_, body) = function(&fixture, &hir, "object");
    assert_eq!(
        ty_text(&hir, expr_ty(&results, tail(&hir, body))),
        "*raw ()"
    );
}

#[test]
fn copy_accepts_a_read_only_source() {
    check_clean(
        "func f(destination: *raw mut i32, source: *raw i32, count: usize) { unsafe { destination.copy(source, count); } }",
    );
}
