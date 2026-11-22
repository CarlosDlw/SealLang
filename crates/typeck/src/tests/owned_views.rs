//! The `owned_view` builtin: owner place, raw source, and unsafe boundary.

use super::support::{Fixture, check_clean, check_errors, expect_ty, expr_ty, function, tail};

#[test]
fn shared_owned_views_keep_the_source_pointee() {
    let (fixture, hir, results) = check_clean(
        "struct Buffer { pointer: *raw i32 }\n\
         func thin(buffer: *Buffer) -> *i32 { unsafe { owned_view(*buffer, (*buffer).pointer) } }\n\
         struct SliceBuffer { pointer: *raw [i32] }\n\
         func slice(buffer: *SliceBuffer) -> *[i32] {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }\n\
         struct TextBuffer { pointer: *raw str }\n\
         func text(buffer: *TextBuffer) -> *str {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    for (name, expected) in [("thin", "*i32"), ("slice", "*[i32]"), ("text", "*str")] {
        let (_, body) = function(&fixture, &hir, name);
        expect_ty(&hir, expr_ty(&results, tail(&hir, body)), expected);
    }
}

#[test]
fn mutable_owned_views_preserve_exclusive_permission() {
    let (fixture, hir, results) = check_clean(
        "struct Buffer { pointer: *raw mut i32 }\n\
         func thin(buffer: *mut Buffer) -> *mut i32 {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }\n\
         struct SliceBuffer { pointer: *raw mut [i32] }\n\
         func slice(buffer: *mut SliceBuffer) -> *mut [i32] {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }\n\
         struct TextBuffer { pointer: *raw mut str }\n\
         func text(buffer: *mut TextBuffer) -> *mut str {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    for (name, expected) in [
        ("thin", "*mut i32"),
        ("slice", "*mut [i32]"),
        ("text", "*mut str"),
    ] {
        let (_, body) = function(&fixture, &hir, name);
        expect_ty(&hir, expr_ty(&results, tail(&hir, body)), expected);
    }
}

#[test]
fn an_owned_view_requires_unsafe() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *Buffer) -> *i32 { owned_view(*buffer, (*buffer).pointer) }",
    );
    fixture.expect("E3070", "requires an `unsafe` block");
}

#[test]
fn the_owner_must_be_an_initialized_place() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw i32 }\n\
         func make() -> Buffer { Buffer { pointer: 0usize as *raw i32 } }\n\
         func f(pointer: *raw i32) -> *i32 { unsafe { owned_view(make(), pointer) } }",
    );
    fixture.expect("E3106", "must be an owner place");
}

#[test]
fn a_temporary_reference_deref_is_a_checked_owner() {
    check_clean(
        "struct Buffer { pointer: *raw i32 }\n\
         func identity(buffer: *Buffer) -> *Buffer { buffer }\n\
         func f(buffer: *Buffer) -> *i32 {\n\
           let source = identity(buffer);\n\
           unsafe { owned_view(*identity(buffer), (*source).pointer) }\n\
         }",
    );
}

#[test]
fn a_temporary_raw_deref_cannot_be_the_owner() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw i32 }\n\
         func identity(buffer: *raw Buffer) -> *raw Buffer { buffer }\n\
         func f(buffer: *raw Buffer) -> *i32 {\n\
           let source = identity(buffer);\n\
           unsafe { owned_view(*identity(buffer), (*source).pointer) }\n\
         }",
    );
    fixture.expect("E3106", "reached through a raw pointer");
}

#[test]
fn a_constant_cannot_own_a_view() {
    let (fixture, _, _) = check_errors(
        "const VALUE: i32 = 1;\n\
         func f(pointer: *raw i32) -> *i32 { unsafe { owned_view(VALUE, pointer) } }",
    );
    fixture.expect("E3106", "constant cannot own");
}

#[test]
fn a_raw_pointer_cannot_be_the_owner() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *raw Buffer) -> *i32 {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    fixture.expect("E3106", "reached through a raw pointer");
}

#[test]
fn the_source_must_be_a_raw_pointer() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *Buffer) -> *i32 { unsafe { owned_view(*buffer, 1) } }",
    );
    fixture.expect("E3106", "must be a raw pointer");
}

#[test]
fn trait_object_pointees_are_reserved() {
    let (fixture, _, _) = check_errors(
        "trait Shape { func area(self: *Self) -> i32; }\n\
         struct Square { side: i32 }\n\
         struct Buffer { pointer: *raw dyn Shape }\n\
         func f(buffer: *Buffer) -> *dyn Shape {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    fixture.expect("E3106", "unsupported pointee type");
}

#[test]
fn an_exclusive_view_requires_a_writable_owner() {
    let (fixture, _, _) = check_errors(
        "struct Buffer { pointer: *raw mut i32 }\n\
         func f(buffer: *Buffer) -> *mut i32 {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    fixture.expect("E3017", "shared reference");
}

#[test]
fn an_owned_view_takes_exactly_two_arguments_and_no_type_arguments() {
    // Arity and type-argument errors leave intentionally unreachable argument
    // nodes in HIR, so check them without the structural invariant helper.
    let fixture = Fixture::new(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *Buffer) -> *i32 { unsafe { owned_view(*buffer) } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3008", "exactly two arguments");

    let fixture = Fixture::new(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *Buffer) -> *i32 {\n\
           unsafe { owned_view::<i32>(*buffer, (*buffer).pointer) }\n\
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E2053", "not generic");
}

#[test]
fn a_user_function_named_owned_view_shadows_the_builtin() {
    check_clean(
        "struct Buffer { pointer: *raw i32 }\n\
         func owned_view(buffer: *Buffer, pointer: *raw i32) -> *i32 {\n\
           unsafe { &*pointer }\n\
         }\n\
         func f(buffer: *Buffer) -> *i32 { owned_view(buffer, (*buffer).pointer) }",
    );
}

#[test]
fn a_fat_raw_pointer_cannot_be_borrowed_without_an_owner() {
    for source in [
        "func f(pointer: *raw [i32]) -> *[i32] { unsafe { &*pointer } }",
        "func f(pointer: *raw [i32]) -> *mut [i32] { unsafe { &mut *pointer } }",
        "func f(pointer: *raw str) -> *str { unsafe { &*pointer } }",
        "func f(pointer: *raw mut str) -> *mut str { unsafe { &mut *pointer } }",
        "trait Shape { func area(self: *Self) -> i32; }\n\
         func f(pointer: *raw dyn Shape) -> *dyn Shape { unsafe { &*pointer } }",
        "trait Shape { func area(self: *Self) -> i32; }\n\
         func f(pointer: *raw mut dyn Shape) -> *mut dyn Shape { unsafe { &mut *pointer } }",
    ] {
        let (fixture, _, _) = check_errors(source);
        fixture.expect("E3106", "fat raw pointer");
    }
}

#[test]
fn a_thin_raw_pointer_still_borrows_normally() {
    check_clean("func f(pointer: *raw i32) -> *i32 { unsafe { &*pointer } }");
}
