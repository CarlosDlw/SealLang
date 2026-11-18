//! Custom iteration through the `IntoIterator` and `Iterator` protocols.

use seal_hir::{ForSource, Hir, HirExprId, StmtKind};

use crate::{MethodKind, ReceiverAdjust};

use super::support::{check_clean, check_errors, function, statements, ty_text_with};

const CHAIN: &str = "\
struct Source { value: i32 } \
struct Cursor { value: i32 } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*self).value < 3 } \
    func next(self: *mut Cursor) -> i32 { \
        let value = (*self).value; \
        (*self).value += 1; \
        value \
    } \
} \
impl IntoIterator<Cursor, i32> for Source { \
    func iter(self: Source) -> Cursor { Cursor { value: self.value } } \
}";

fn for_source(hir: &Hir, body: &seal_hir::Body) -> HirExprId {
    statements(hir, body)
        .iter()
        .find_map(|statement| match hir.stmt(*statement).kind.clone() {
            StmtKind::For {
                source: ForSource::Expr(source),
                ..
            } => Some(source),
            _ => None,
        })
        .expect("the function contains a for expression source")
}

#[test]
fn a_custom_source_records_the_into_iterator_and_iterator_calls() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 3 }}; \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let iterable = for_source(&hir, body);
    let protocol = results
        .iteration(iterable)
        .expect("the custom source records a protocol");

    assert_eq!(ty_text_with(&fixture, &hir, protocol.item_ty), "i32");
    assert_eq!(ty_text_with(&fixture, &hir, protocol.iterator_ty), "Cursor");
    assert!(!protocol.direct_iterator);
    let iter = protocol.iter.as_ref().expect("IntoIterator supplies iter");
    assert!(matches!(
        iter.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Value,
            ..
        }
    ));
    assert_eq!(iter.method.receiver_adjust, ReceiverAdjust::None);
    assert_eq!(ty_text_with(&fixture, &hir, iter.method.ret_ty), "Cursor");
    assert!(matches!(
        protocol.has_next.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
    assert_eq!(
        protocol.has_next.method.receiver_adjust,
        ReceiverAdjust::SharedBorrow
    );
    assert!(matches!(
        protocol.next.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Exclusive,
            ..
        }
    ));
    assert_eq!(
        protocol.next.method.receiver_adjust,
        ReceiverAdjust::ExclusiveBorrow
    );
    assert!(iter.instance.is_some());
    assert!(protocol.next.instance.is_some());
}

#[test]
fn a_borrowed_source_keeps_the_collection_available() {
    let source = format!(
        "{CHAIN} \
         impl IntoIterator<Cursor, i32> for *Source {{ \
             func iter(self: *Source) -> Cursor {{ Cursor {{ value: (*self).value }} }} \
         }} \
         func main() -> i32 {{ \
           let source = Source {{ value: 4 }}; \
           let mut total = 0; \
           for value in &source {{ total += value; }} \
           total + source.value \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let iterable = for_source(&hir, body);
    let protocol = results
        .iteration(iterable)
        .expect("the borrowed source records a protocol");
    assert_eq!(ty_text_with(&fixture, &hir, protocol.item_ty), "i32");
    assert_eq!(ty_text_with(&fixture, &hir, protocol.iterator_ty), "Cursor");
}

#[test]
fn an_exclusive_borrowed_source_is_supported() {
    let source = format!(
        "{CHAIN} \
         impl IntoIterator<Cursor, i32> for *mut Source {{ \
             func iter(self: *mut Source) -> Cursor {{ Cursor {{ value: (*self).value }} }} \
         }} \
         func main() -> i32 {{ \
           let mut source = Source {{ value: 5 }}; \
           let mut total = 0; \
           for value in &mut source {{ total += value; }} \
           total \
         }}"
    );
    check_clean(&source);
}

#[test]
fn a_direct_iterator_source_uses_the_fallback_without_iter() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let cursor = Cursor {{ value: 0 }}; \
           let mut total = 0; \
           for value in cursor {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let iterable = for_source(&hir, body);
    let protocol = results
        .iteration(iterable)
        .expect("the direct iterator records a protocol");
    assert!(protocol.direct_iterator);
    assert!(protocol.iter.is_none());
    assert_eq!(ty_text_with(&fixture, &hir, protocol.iterator_ty), "Cursor");
    assert_eq!(ty_text_with(&fixture, &hir, protocol.item_ty), "i32");
}

#[test]
fn generic_bounds_resolve_into_iterator_and_direct_iterator() {
    let source = format!(
        "{CHAIN} \
         func chained<S: IntoIterator<Cursor, i32>>(source: S) -> i32 {{ \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }} \
         func direct<I: Iterator<i32>>(iter: I) -> i32 {{ \
           let mut total = 0; \
           for value in iter {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, results) = check_clean(&source);

    let (_, body) = function(&fixture, &hir, "chained");
    let iterable = for_source(&hir, body);
    let protocol = results
        .iteration(iterable)
        .expect("the bound records a protocol");
    let iter = protocol.iter.as_ref().expect("IntoIterator supplies iter");
    assert!(matches!(iter.method.kind, MethodKind::TraitBound { .. }));
    assert!(iter.instance.is_none());
    assert!(matches!(
        protocol.next.method.kind,
        MethodKind::Trait { .. }
    ));

    let (_, body) = function(&fixture, &hir, "direct");
    let iterable = for_source(&hir, body);
    let protocol = results
        .iteration(iterable)
        .expect("the direct bound records a protocol");
    assert!(protocol.direct_iterator);
    assert!(matches!(
        protocol.next.method.kind,
        MethodKind::TraitBound { .. }
    ));
    assert!(protocol.next.instance.is_none());
}

#[test]
fn empty_and_refutable_bodies_are_checked_normally() {
    check_clean(&format!(
        "{CHAIN} func main() {{ let source = Source {{ value: 0 }}; for value in source {{ }} }}"
    ));

    let source = format!(
        "{CHAIN} func main() {{ let source = Source {{ value: 0 }}; for 0 in source {{ }} }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3034", "does not match every element");
}

#[test]
fn break_continue_and_non_copy_items_are_accepted() {
    let source = format!(
        "{CHAIN} \
         struct Item {{ value: i32 }} \
         struct Items {{ value: i32 }} \
         struct ItemCursor {{ value: i32 }} \
         impl Iterator<Item> for ItemCursor {{ \
             func has_next(self: *ItemCursor) -> bool {{ (*self).value < 2 }} \
             func next(self: *mut ItemCursor) -> Item {{ \
                 (*self).value += 1; \
                 Item {{ value: (*self).value }} \
             }} \
         }} \
         impl IntoIterator<ItemCursor, Item> for Items {{ \
             func iter(self: Items) -> ItemCursor {{ ItemCursor {{ value: 0 }} }} \
         }} \
         func main() -> i32 {{ \
           let items = Items {{ value: 2 }}; \
           let mut total = 0; \
           for item in items {{ \
               if item.value == 1 {{ continue; }} \
               let moved = item; \
               total += moved.value; \
               if total > 2 {{ break; }} \
           }} \
           total \
         }}"
    );
    check_clean(&source);
}

#[test]
fn a_source_without_any_iteration_protocol_is_e3035() {
    let source = "struct Source { value: i32 } \
                  func main() { let source = Source { value: 1 }; for value in source { let _ = value; } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3035", "cannot iterate over");
    let diagnostic = fixture
        .diagnostics()
        .into_iter()
        .find(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E3035"))
        .expect("E3035 is reported");
    assert!(
        diagnostic
            .helps()
            .iter()
            .any(|help| help.contains("IntoIterator")),
        "the help names the accepted protocols: {:?}",
        diagnostic.helps()
    );
}

#[test]
fn an_iterator_type_without_iterator_is_e3051() {
    let source = "\
        struct Source { value: i32 } \
        struct Cursor { value: i32 } \
        impl IntoIterator<Cursor, i32> for Source { \
            func iter(self: Source) -> Cursor { Cursor { value: 0 } } \
        } \
        func main() { let source = Source { value: 1 }; for value in source { let _ = value; } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3051", "does not implement `Iterator<i32>`");
}

#[test]
fn two_into_iterator_candidates_are_e3050() {
    let source = "\
        struct Source { value: i32 } \
        struct First { value: i32 } \
        struct Second { value: i32 } \
        impl Iterator<i32> for First { \
            func has_next(self: *First) -> bool { true } \
            func next(self: *mut First) -> i32 { 0 } \
        } \
        impl Iterator<i32> for Second { \
            func has_next(self: *Second) -> bool { true } \
            func next(self: *mut Second) -> i32 { 0 } \
        } \
        impl IntoIterator<First, i32> for Source { \
            func iter(self: Source) -> First { First { value: 0 } } \
        } \
        impl IntoIterator<Second, i32> for Source { \
            func iter(self: Source) -> Second { Second { value: 0 } } \
        } \
        func main() { let source = Source { value: 1 }; for value in source { let _ = value; } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3050", "cannot choose an `IntoIterator` implementation");
}

#[test]
fn two_iterator_candidates_are_e3052() {
    let source = "\
        struct Source { value: i32 } \
        impl Iterator<i32> for Source { \
            func has_next(self: *Source) -> bool { true } \
            func next(self: *mut Source) -> i32 { 0 } \
        } \
        impl Iterator<bool> for Source { \
            func has_next(self: *Source) -> bool { true } \
            func next(self: *mut Source) -> bool { false } \
        } \
        func main() { let source = Source { value: 1 }; for value in source { let _ = value; } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3052", "cannot choose an `Iterator` implementation");
}

#[test]
fn the_body_can_infer_the_item_type() {
    let source = "\
        struct Source { value: i32 } \
        struct Cursor { value: i32 } \
        impl<T> Iterator<T> for Cursor { \
            func has_next(self: *Cursor) -> bool { false } \
            func next(self: *mut Cursor) -> T { produce() } \
        } \
        impl<T> IntoIterator<Cursor, T> for Source { \
            func iter(self: Source) -> Cursor { Cursor { value: 0 } } \
        } \
        func produce<T>() -> T { produce() } \
        func main() -> i32 { \
            let source = Source { value: 1 }; \
            let mut total = 0; \
            for value in source { let numeric: i32 = value; total += numeric; } \
            total \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let iterable = for_source(&hir, body);
    let protocol = results.iteration(iterable).expect("protocol is recorded");
    assert_eq!(ty_text_with(&fixture, &hir, protocol.item_ty), "i32");
}

#[test]
fn an_unconstrained_item_type_is_e3055() {
    let source = "\
        struct Source { value: i32 } \
        struct Cursor { value: i32 } \
        impl<T> Iterator<T> for Cursor { \
            func has_next(self: *Cursor) -> bool { false } \
            func next(self: *mut Cursor) -> T { produce() } \
        } \
        impl<T> IntoIterator<Cursor, T> for Source { \
            func iter(self: Source) -> Cursor { Cursor { value: 0 } } \
        } \
        func produce<T>() -> T { produce() } \
        func main() { \
            let source = Source { value: 1 }; \
            for value in source { let _ = value; } \
        }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3055", "cannot infer");
}

#[test]
fn an_unconstrained_iterator_type_is_e3055() {
    let source = "\
        struct Source { value: i32 } \
        impl<I, T> IntoIterator<I, T> for Source { \
            func iter(self: Source) -> I { self.iter() } \
        } \
        func main() { let source = Source { value: 1 }; for value in source { let _ = value; } }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3055", "cannot infer");
}

#[test]
fn a_dynamic_into_iterator_is_rejected() {
    let source = "\
        struct Source { value: i32 } \
        func main() { \
            let source = Source { value: 1 }; \
            let dynamic: *dyn IntoIterator<Source, i32> = &source; \
            for value in dynamic { let _ = value; } \
        }";
    let (fixture, _, _) = check_errors(source);
    assert!(
        fixture
            .codes()
            .iter()
            .any(|code| code == "E3035" || code == "E2065"),
        "expected the dynamic iteration source to be rejected, got {:?}",
        fixture.codes()
    );
}

#[test]
fn builtin_ranges_and_arrays_do_not_record_a_protocol() {
    let source = "\
        func main(values: [i32; 2]) { \
            for value in values { let _ = value; } \
            for index in 0..2 { let _ = index; } \
            for value in &values { let _ = value; } \
        }";
    let (_, hir, results) = check_clean(source);
    for body in hir.bodies().iter() {
        for statement in &hir.block(body.root).statements {
            if let StmtKind::For {
                source: ForSource::Expr(source),
                ..
            } = hir.stmt(*statement).kind
            {
                assert!(
                    results.iteration(source).is_none(),
                    "built-in iteration must not record a custom protocol"
                );
            }
        }
    }
}

#[test]
fn a_reference_layer_supplies_the_iterator_implementation() {
    let source = "\
        struct Source { cursor: Cursor } \
        struct Cursor { value: i32 } \
        impl Iterator<i32> for Cursor { \
            func has_next(self: *Cursor) -> bool { (*self).value < 1 } \
            func next(self: *mut Cursor) -> i32 { (*self).value } \
        } \
        impl IntoIterator<*mut Cursor, i32> for *mut Source { \
            func iter(self: *mut Source) -> *mut Cursor { &mut (*self).cursor } \
        } \
        func main() -> i32 { \
            let mut source = Source { cursor: Cursor { value: 1 } }; \
            let mut total = 0; \
            for value in &mut source { total += value; } \
            total \
        }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let iterable = for_source(&hir, body);
    let protocol = results.iteration(iterable).expect("protocol is recorded");
    assert_eq!(
        ty_text_with(&fixture, &hir, protocol.iterator_ty),
        "*mut Cursor"
    );
    assert!(matches!(
        protocol.has_next.method.receiver_adjust,
        ReceiverAdjust::SharedReborrow
    ));
    assert_eq!(protocol.next.method.receiver_adjust, ReceiverAdjust::None);
    assert!(matches!(
        protocol.next.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Exclusive,
            ..
        }
    ));
    assert_eq!(protocol.next.method.receiver_derefs, 0);
    assert!(matches!(
        protocol.has_next.method.kind,
        MethodKind::Trait {
            receiver: seal_hir::ReceiverKind::Shared,
            ..
        }
    ));
}
