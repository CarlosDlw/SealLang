//! The compiler-provided iteration and indexing protocol definitions.

use seal_def::DefKind;
use seal_ty::{RefKind, TraitDef, TraitMethodDef, TyKind};

use crate::{Hir, Protocol, ProtocolMethod, TraitMethodLookup};

use super::support::{Fixture, function, lower_clean};

fn trait_def(hir: &Hir, protocol: Protocol) -> TraitDef {
    hir.trait_definition(hir.protocol_trait(protocol))
        .expect("protocol trait has a semantic definition")
}

fn method<'a>(fixture: &Fixture, definition: &'a TraitDef, name: &str) -> &'a TraitMethodDef {
    definition
        .method(fixture.interner.intern(name))
        .unwrap_or_else(|| panic!("trait is missing `{name}`"))
}

fn assert_param(hir: &Hir, definition: &TraitDef, ty: seal_ty::TyId, index: usize) {
    let expected = definition.generics.params[index].param;
    match hir.ty_ctxt().kind(ty) {
        TyKind::Param(param) => assert_eq!(param, expected),
        other => panic!("expected parameter at index {index}, got {other:?}"),
    }
}

#[test]
fn installs_every_protocol_with_its_methods() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let expected = [
        (Protocol::Iterator, &["has_next", "next"][..]),
        (Protocol::IntoIterator, &["iter"][..]),
        (Protocol::Index, &["index"][..]),
        (Protocol::IndexMut, &["index_mut"][..]),
    ];
    for (protocol, names) in expected {
        let definition = trait_def(&hir, protocol);
        assert_eq!(definition.name, fixture.interner.intern(protocol.name()));
        assert_eq!(definition.methods.len(), names.len());
        assert_eq!(
            definition
                .methods
                .iter()
                .map(|method| method.name)
                .collect::<Vec<_>>(),
            names
                .iter()
                .map(|name| fixture.interner.intern(name))
                .collect::<Vec<_>>()
        );
        assert!(definition.supertraits.is_empty());
        assert!(definition.generics.params[0].is_self);
        assert_eq!(
            definition.generics.params.len(),
            match protocol {
                Protocol::Iterator => 2,
                Protocol::IntoIterator | Protocol::Index | Protocol::IndexMut => 3,
            }
        );
        assert_eq!(
            hir.trait_def(fixture.interner.intern(protocol.name())),
            Some(definition.def)
        );
        assert!(hir.is_builtin_def(definition.def));
        assert!(hir.is_builtin_def(definition.generics.params[0].def));

        for name in names {
            let method = method(&fixture, &definition, name);
            assert_eq!(method.name, fixture.interner.intern(name));
            assert!(!method.has_body);
            assert!(hir.is_builtin_def(method.def));
            assert_eq!(
                hir.protocol_method_of(method.def),
                Some((protocol, protocol_method(protocol, name)))
            );
        }
    }
}

fn protocol_method(protocol: Protocol, name: &str) -> ProtocolMethod {
    match (protocol, name) {
        (Protocol::Iterator, "has_next") => ProtocolMethod::HasNext,
        (Protocol::Iterator, "next") => ProtocolMethod::Next,
        (Protocol::IntoIterator, "iter") => ProtocolMethod::Iter,
        (Protocol::Index, "index") => ProtocolMethod::Index,
        (Protocol::IndexMut, "index_mut") => ProtocolMethod::IndexMut,
        _ => panic!("unexpected protocol method `{name}`"),
    }
}

#[test]
fn iterator_signatures_use_references_and_the_item_parameter() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let definition = trait_def(&hir, Protocol::Iterator);
    let has_next = method(&fixture, &definition, "has_next");
    let next = method(&fixture, &definition, "next");

    assert_eq!(
        has_next.receiver,
        Some(seal_ty::TraitMethodReceiver {
            reference: Some(RefKind::Shared)
        })
    );
    assert!(has_next.signature.params.is_empty());
    assert!(matches!(
        hir.ty_ctxt().kind(has_next.signature.ret),
        TyKind::Bool
    ));

    assert_eq!(
        next.receiver,
        Some(seal_ty::TraitMethodReceiver {
            reference: Some(RefKind::Exclusive)
        })
    );
    assert!(next.signature.params.is_empty());
    assert_param(&hir, &definition, next.signature.ret, 1);
}

#[test]
fn into_iterator_signatures_take_self_by_value() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let definition = trait_def(&hir, Protocol::IntoIterator);
    let iter = method(&fixture, &definition, "iter");

    assert_eq!(
        iter.receiver,
        Some(seal_ty::TraitMethodReceiver { reference: None })
    );
    assert!(iter.signature.params.is_empty());
    assert_param(&hir, &definition, iter.signature.ret, 1);
}

#[test]
fn index_signatures_return_references_to_the_element_parameter() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    for (protocol, method_name, kind) in [
        (Protocol::Index, "index", RefKind::Shared),
        (Protocol::IndexMut, "index_mut", RefKind::Exclusive),
    ] {
        let definition = trait_def(&hir, protocol);
        let index = method(&fixture, &definition, method_name);
        assert_eq!(
            index.receiver,
            Some(seal_ty::TraitMethodReceiver {
                reference: Some(kind)
            })
        );
        assert_eq!(index.signature.params.len(), 1);
        assert_param(&hir, &definition, index.signature.params[0], 1);
        match hir.ty_ctxt().kind(index.signature.ret) {
            TyKind::Ref {
                kind: return_kind,
                pointee,
            } => {
                assert_eq!(return_kind, kind);
                assert_param(&hir, &definition, pointee, 2);
            }
            other => panic!("expected a pointer return, got {other:?}"),
        }
    }
}

#[test]
fn protocol_traits_are_available_to_impls_and_bounds() {
    let (fixture, hir) = lower_clean(
        "struct Cursor { value: i32 } \
         impl Iterator<i32> for Cursor { \
             func has_next(self: *Cursor) -> bool { true } \
             func next(self: *mut Cursor) -> i32 { (*self).value } \
         } \
         func consume<X>(cursor: X) -> i32 where X: Iterator<i32> { 0 }",
    );
    let iterator = hir.protocol_trait(Protocol::Iterator);
    assert_eq!(hir.definitions().kind(iterator), DefKind::Trait);
    let implementations = hir.trait_impls();
    assert_eq!(implementations.len(), 1);
    assert_eq!(implementations[0].trait_ref.trait_def, iterator);

    let cursor = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            data.kind == DefKind::Struct && fixture.interner.resolve(data.name) == "Cursor"
        })
        .map(|(def, _)| def)
        .expect("Cursor is registered");
    let TraitMethodLookup::Found {
        trait_def,
        function,
        ..
    } = hir.trait_method(cursor, fixture.interner.intern("next"))
    else {
        panic!("the protocol method is available through the impl");
    };
    assert_eq!(trait_def, iterator);
    assert_eq!(hir.definitions().get(function).parent, Some(cursor));
}

#[test]
fn every_protocol_accepts_a_conforming_custom_implementation() {
    let (_fixture, hir) = lower_clean(
        "struct Holder { value: i32 } \
         impl Iterator<i32> for Holder { \
             func has_next(self: *Holder) -> bool { true } \
             func next(self: *mut Holder) -> i32 { (*self).value } \
         } \
         impl IntoIterator<Holder, i32> for Holder { \
             func iter(self: Holder) -> Holder { self } \
         } \
         impl Index<usize, i32> for Holder { \
             func index(self: *Holder, index: usize) -> *i32 { &(*self).value } \
         } \
         impl IndexMut<usize, i32> for Holder { \
             func index_mut(self: *mut Holder, index: usize) -> *mut i32 { &mut (*self).value } \
         }",
    );
    assert_eq!(hir.trait_impls().len(), 4);
}

#[test]
fn protocol_impls_with_different_trait_arguments_are_distinct() {
    let (_fixture, hir) = lower_clean(
        "struct Holder { value: i32 } \
         impl Index<usize, i32> for Holder { \
             func index(self: *Holder, index: usize) -> *i32 { &(*self).value } \
         } \
         impl Index<bool, i32> for Holder { \
             func index(self: *Holder, index: bool) -> *i32 { &(*self).value } \
         }",
    );
    assert_eq!(hir.trait_impls().len(), 2);
}

#[test]
fn protocol_impls_for_custom_references_and_generic_targets_are_valid() {
    let (_fixture, hir) = lower_clean(
        "struct Holder<T> { value: T } \
         impl<T> Index<usize, T> for Holder<T> { \
             func index(self: *Holder<T>, index: usize) -> *T { 0 } \
         } \
         struct Cell { value: i32 } \
         impl Index<usize, i32> for *Cell { \
             func index(self: **Cell, index: usize) -> *i32 { 0 } \
         }",
    );
    assert_eq!(hir.trait_impls().len(), 2);
}

#[test]
fn a_protocol_impl_must_supply_every_method() {
    let fixture = Fixture::new(
        "struct Holder { value: i32 } \
         impl Iterator<i32> for Holder { \
             func has_next(self: *Holder) -> bool { false } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2022", "requires `next`");
}

#[test]
fn a_protocol_impl_may_not_add_methods() {
    let fixture = Fixture::new(
        "struct Holder { value: i32 } \
         impl Iterator<i32> for Holder { \
             func has_next(self: *Holder) -> bool { false } \
             func next(self: *mut Holder) -> i32 { 0 } \
             func extra(self: *Holder) -> i32 { 0 } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2023", "does not declare `extra`");
}

#[test]
fn protocol_receivers_must_match_the_built_in_declaration() {
    let fixture = Fixture::new(
        "struct Holder { value: i32 } \
         impl Iterator<i32> for Holder { \
             func has_next(self: *mut Holder) -> bool { false } \
             func next(self: *Holder) -> i32 { 0 } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "receiver does not match");
}

#[test]
fn protocol_parameters_and_returns_are_checked_against_the_built_in() {
    let fixture = Fixture::new(
        "struct Holder { value: i32 } \
         impl Index<usize, i32> for Holder { \
             func index(self: *Holder, index: i64) -> *i32 { 0 } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "parameter 1 has type `i64`");

    let fixture = Fixture::new(
        "struct Holder { value: i32 } \
         impl Iterator<i32> for Holder { \
             func has_next(self: *Holder) -> i32 { 0 } \
             func next(self: *mut Holder) -> i32 { 0 } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "return type is `i32`");
}

#[test]
fn protocol_impls_for_compiler_owned_sequences_are_rejected() {
    for protocol in Protocol::ALL {
        let arguments = match protocol {
            Protocol::Iterator => "<i32>",
            Protocol::IntoIterator => "<i32, i32>",
            Protocol::Index | Protocol::IndexMut => "<usize, i32>",
        };
        for target in ["[i32; 2]", "[i32]", "*[i32]", "*mut [i32]", "*mut [i32; 2]"] {
            let source = format!("impl {}{arguments} for {target} {{}}", protocol.name());
            let fixture = Fixture::new(&source);
            let _ = fixture.lower();
            fixture.expect("E2067", "compiler-owned type");
            assert_eq!(fixture.codes(), ["E2067"], "source: {source}");
        }
    }
}

#[test]
fn a_source_trait_cannot_reuse_a_protocol_name() {
    for protocol in Protocol::ALL {
        let source = format!("trait {} {{ }}", protocol.name());
        let fixture = Fixture::new(&source);
        let _ = fixture.lower();
        fixture.expect("E2066", "reserved");
        assert_eq!(fixture.codes(), ["E2066"]);
    }
}

#[test]
fn source_definitions_are_not_marked_as_builtins() {
    let (fixture, hir) = lower_clean("func main() -> i32 { 0 }");
    let (main_def, _) = function(&fixture, &hir, "main");
    assert!(!hir.is_builtin_def(main_def));
}
