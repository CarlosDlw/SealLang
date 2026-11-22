//! Type checking for inherent `impl str` and the string method catalog.

use seal_methods::MethodOwner;

use crate::{MethodKind, ReceiverForm};

use super::support::{
    check_clean, check_core_clean, check_errors, expr_ty, function, tail, ty_text,
};

#[test]
fn len_resolves_through_the_string_catalog() {
    let (fixture, hir, results) = check_clean("func length(text: *str) -> usize { text.len() }");
    let (_, body) = function(&fixture, &hir, "length");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("str length is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::Intrinsic(id) if id.def().owner == MethodOwner::Str
    ));
    assert_eq!(resolved.receiver_form, ReceiverForm::Value);
    assert_eq!(ty_text(&hir, expr_ty(&results, call)), "usize");
}

#[test]
fn is_empty_resolves_through_the_inherent_registry() {
    let (fixture, hir, results) = check_core_clean(
        "impl str { \
             public func is_empty(self: *str) -> bool { self.len() == 0 } \
         } \
         func main() -> bool { \
             let text: *str = \"hello\"; \
             text.is_empty() \
         }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("is_empty is resolved");
    let MethodKind::User { function, receiver } = resolved.kind else {
        panic!("expected an inherent str method, got {:?}", resolved.kind);
    };
    assert_eq!(fixture.symbol(hir.definitions().name(function)), "is_empty");
    assert_eq!(receiver, seal_hir::ReceiverKind::Shared);
    assert_eq!(ty_text(&hir, expr_ty(&results, call)), "bool");
}

#[test]
fn an_unknown_str_method_is_reported() {
    let (fixture, _, _) = check_errors("func f(text: *str) -> bool { text.nope() }");
    fixture.expect("E3027", "no method named `nope`");
}

#[test]
fn catalog_string_methods_have_their_documented_result_types() {
    let (fixture, hir, results) = check_clean(
        "func slice(text: *str) -> *str { text.slice(0, 1) } \
         func bytes(text: *str) -> *[u8] { text.as_bytes() } \
         func at(text: *str) -> char { text.char_at(0) } \
         func boundary(text: *str) -> bool { text.is_char_boundary(0) }",
    );
    for (name, expected) in [
        ("slice", "*str"),
        ("bytes", "*[u8]"),
        ("at", "char"),
        ("boundary", "bool"),
    ] {
        let (_, body) = function(&fixture, &hir, name);
        assert_eq!(ty_text(&hir, expr_ty(&results, tail(&hir, body))), expected);
    }
}

#[test]
fn as_str_unchecked_requires_unsafe_and_a_byte_slice() {
    let (fixture, _, _) = check_errors("func f(bytes: *[u8]) -> *str { bytes.as_str_unchecked() }");
    fixture.expect("E3070", "requires an `unsafe` block");

    let (fixture, _, _) =
        check_errors("func f(bytes: *[i32]) -> *str { unsafe { bytes.as_str_unchecked() } }");
    fixture.expect("E3075", "requires a byte slice");

    check_clean("func f(bytes: *[u8]) -> *str { unsafe { bytes.as_str_unchecked() } }");
}
