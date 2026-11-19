//! Unsafe function call gates and explicit unsafe bodies.

use super::support::{check_clean, check_errors, function};

#[test]
fn an_unsafe_call_requires_an_unsafe_block() {
    let (fixture, _, _) = check_errors("unsafe func danger() {} func main() { danger(); }");
    fixture.expect("E3070", "calling `danger` requires an `unsafe` block");
}

#[test]
fn an_unsafe_call_inside_an_unsafe_block_is_allowed() {
    let (fixture, _, _) =
        check_clean("unsafe func danger() {} func main() { unsafe { danger(); } }");
    let _ = fixture;
}

#[test]
fn a_safe_call_inside_an_unsafe_block_is_allowed() {
    let (fixture, _, _) = check_clean("func safe() {} func main() { unsafe { safe(); } }");
    let _ = fixture;
}

#[test]
fn an_unsafe_function_body_is_not_an_implicit_unsafe_context() {
    let (fixture, _, _) =
        check_errors("unsafe func read(pointer: *raw i32) -> i32 { *pointer } func main() {}");
    fixture.expect("E3070", "requires an `unsafe` block");
}

#[test]
fn an_unsafe_function_body_can_open_its_own_unsafe_block() {
    let (fixture, _, _) = check_clean(
        "unsafe func read(pointer: *raw i32) -> i32 { unsafe { *pointer } } func main() {}",
    );
    let _ = fixture;
}

#[test]
fn an_inherent_unsafe_method_is_gated_at_the_call() {
    let source = "\
        struct Token { value: i32 } \
        impl Token { unsafe func bump(self: *mut Token) { unsafe { (*self).value = (*self).value + 1; } } } \
        func main() { let mut token = Token { value: 0 }; token.bump(); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "calling `bump` requires an `unsafe` block");
}

#[test]
fn an_inherent_unsafe_method_can_be_called_inside_unsafe() {
    let source = "\
        struct Token { value: i32 } \
        impl Token { unsafe func bump(self: *mut Token) { unsafe { (*self).value = (*self).value + 1; } } } \
        func main() { let mut token = Token { value: 0 }; unsafe { token.bump(); } }";
    check_clean(source);
}

#[test]
fn a_trait_unsafe_method_is_gated_at_the_call() {
    let source = "\
        trait Access { unsafe func read(self: *Self) -> i32; } \
        struct Token { value: i32 } \
        impl Access for Token { unsafe func read(self: *Token) -> i32 { unsafe { (*self).value } } } \
        func main() -> i32 { let token = Token { value: 1 }; token.read() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "calling `read` requires an `unsafe` block");
}

#[test]
fn a_trait_unsafe_method_can_be_called_inside_unsafe() {
    let source = "\
        trait Access { unsafe func read(self: *Self) -> i32; } \
        struct Token { value: i32 } \
        impl Access for Token { unsafe func read(self: *Token) -> i32 { unsafe { (*self).value } } } \
        func main() -> i32 { let token = Token { value: 1 }; unsafe { token.read() } }";
    check_clean(source);
}

#[test]
fn an_unsafe_associated_function_is_gated() {
    let source = "\
        struct Token { value: i32 } \
        impl Token { unsafe func make() -> Token { Token { value: 1 } } } \
        func main() { Token::make(); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "calling `make` requires an `unsafe` block");
}

#[test]
fn an_unsafe_function_may_recurse_inside_unsafe() {
    let source = "\
        unsafe func countdown(value: i32) -> i32 { \
            if value <= 0 { 0 } else { unsafe { countdown(value - 1) } } \
        } \
        func main() -> i32 { unsafe { countdown(3) } }";
    let (fixture, hir, _) = check_clean(source);
    let (countdown, body) = function(&fixture, &hir, "countdown");
    assert!(body.is_unsafe);
    assert!(hir.is_unsafe_function(countdown));
}

#[test]
fn a_const_unsafe_function_can_be_called_in_a_const_unsafe_block() {
    let source = "\
        const unsafe func twice(value: i32) -> i32 { value * 2 } \
        const VALUE: i32 = unsafe { twice(2) }; \
        func main() -> i32 { VALUE }";
    check_clean(source);
}

#[test]
fn a_generic_unsafe_function_is_gated() {
    let source = "\
        unsafe func identity<T>(value: T) -> T { unsafe { value } } \
        func main() -> i32 { identity(1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "calling `identity` requires an `unsafe` block");
}

#[test]
fn a_generic_inherent_unsafe_method_is_gated() {
    let source = "\
        struct Cell<T> { value: T } \
        impl<T> Cell<T> { unsafe func set(self: *mut Cell<T>, value: T) { unsafe { (*self).value = value; } } } \
        func main() { let mut cell = Cell { value: 0 }; cell.set(1); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3070", "calling `set` requires an `unsafe` block");
}

#[test]
fn the_entry_point_cannot_be_unsafe() {
    let (fixture, _, _) = check_errors("unsafe func main() -> i32 { 0 }");
    fixture.expect("E3073", "entry point `main` cannot be `unsafe`");
}
