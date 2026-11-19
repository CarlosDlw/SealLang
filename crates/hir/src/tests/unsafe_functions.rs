//! Unsafe function signatures, trait matching, and destructor rules.

use seal_def::{DefId, DefKind};
use seal_ty::TraitMethodDef;

use crate::Body;

use super::support::{Fixture, function, lower_clean};

fn definition(fixture: &Fixture, hir: &crate::Hir, name: &str, kind: DefKind) -> DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

fn free_body<'a>(fixture: &Fixture, hir: &'a crate::Hir, name: &str) -> (DefId, &'a Body) {
    function(fixture, hir, name)
}

fn trait_method(
    fixture: &Fixture,
    hir: &crate::Hir,
    trait_name: &str,
    method_name: &str,
) -> TraitMethodDef {
    let trait_def = hir
        .trait_def(fixture.interner.intern(trait_name))
        .unwrap_or_else(|| panic!("no trait named `{trait_name}`"));
    hir.trait_definition(trait_def)
        .and_then(|declaration| {
            declaration
                .methods
                .iter()
                .find(|method| fixture.symbol(method.name) == method_name)
                .cloned()
        })
        .unwrap_or_else(|| panic!("no trait method named `{method_name}`"))
}

#[test]
fn unsafe_bodies_and_trait_declarations_keep_the_marker() {
    let source = "\
        trait Access {
            unsafe func read(self: *Self) -> i32;
            unsafe func fallback(self: *Self) -> i32 { unsafe { 1 } }
        }
        struct Token { value: i32 }
        impl Access for Token {
            unsafe func read(self: *Token) -> i32 { unsafe { (*self).value } }
        }
        unsafe func free(value: i32) -> i32 { unsafe { value } }
        const unsafe func constant(value: i32) -> i32 { value }";
    let (fixture, hir) = lower_clean(source);

    let (free, body) = free_body(&fixture, &hir, "free");
    assert!(body.is_unsafe, "the free function body records `unsafe`");
    assert!(!body.is_const);
    assert!(hir.is_unsafe_function(free));

    let (constant, body) = free_body(&fixture, &hir, "constant");
    assert!(body.is_unsafe && body.is_const);
    assert!(hir.is_unsafe_function(constant));

    assert!(trait_method(&fixture, &hir, "Access", "read").is_unsafe);
    assert!(trait_method(&fixture, &hir, "Access", "fallback").is_unsafe);

    let implementation = definition(&fixture, &hir, "read", DefKind::Function);
    assert!(
        hir.body_of(implementation)
            .is_some_and(|body| body.is_unsafe)
    );
}

#[test]
fn a_trait_and_its_impl_must_agree_on_unsafe() {
    let fixture = Fixture::new(
        "trait Access { unsafe func read(self: *Self) -> i32; } \
         struct Token { value: i32 } \
         impl Access for Token { func read(self: *Token) -> i32 { (*self).value } }",
    );
    fixture.lower();
    fixture.expect("E2024", "`unsafe` marker does not match");
}

#[test]
fn an_impl_may_not_add_unsafe_to_a_safe_trait_method() {
    let fixture = Fixture::new(
        "trait Access { func read(self: *Self) -> i32; } \
         struct Token { value: i32 } \
         impl Access for Token { unsafe func read(self: *Token) -> i32 { unsafe { (*self).value } } }",
    );
    fixture.lower();
    fixture.expect("E2024", "`unsafe` marker does not match");
}

#[test]
fn a_destructor_cannot_be_unsafe() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         impl Drop for Token { unsafe func drop(self: *mut Token) { unsafe { (*self).value = 0; } } }",
    );
    fixture.lower();
    fixture.expect("E3072", "cannot be an `unsafe func`");
}
