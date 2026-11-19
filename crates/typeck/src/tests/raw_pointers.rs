//! Raw pointer types: spelling, properties, and unification.

use seal_ty::{IntTy, RawPtrKind, TyKind};

use super::support::{check_clean, check_errors, expect_ty, function, local_ty};

#[test]
fn raw_pointer_parameters_are_copy_sized_and_need_no_drop() {
    let (fixture, hir, results) = check_clean("func f(a: *raw i32, b: *raw mut i32) {}");
    let (_, body) = function(&fixture, &hir, "f");
    let read_only = local_ty(&results, body.params[0].def);
    let writable = local_ty(&results, body.params[1].def);

    expect_ty(&hir, read_only, "*raw i32");
    expect_ty(&hir, writable, "*raw mut i32");
    assert_ne!(read_only, writable);

    let tcx = hir.ty_ctxt();
    assert_eq!(
        tcx.raw_parts(read_only),
        Some((RawPtrKind::Const, tcx.int_ty(IntTy::I32)))
    );
    assert_eq!(
        tcx.raw_parts(writable),
        Some((RawPtrKind::Mut, tcx.int_ty(IntTy::I32)))
    );
    for ty in [read_only, writable] {
        assert!(tcx.is_copy(ty));
        assert!(tcx.is_sized(ty));
        assert!(!tcx.needs_drop(ty));
        assert!(!tcx.contains_reference(ty));
        assert!(tcx.contains_raw_pointer(ty));
    }
}

#[test]
fn raw_pointers_to_unsized_pointees_are_still_sized_values() {
    let (fixture, hir, results) = check_clean(
        "trait Shape {}\n\
         func f(a: *raw str, b: *raw [i32], c: *raw dyn Shape) {}",
    );
    let (_, body) = function(&fixture, &hir, "f");
    for param in &body.params {
        let ty = local_ty(&results, param.def);
        assert!(
            hir.ty_ctxt().is_sized(ty),
            "`{}` should be a sized pointer value",
            hir.ty_ctxt().display(ty)
        );
    }
    let object = local_ty(&results, body.params[2].def);
    assert!(matches!(hir.ty_ctxt().kind(object), TyKind::RawPtr { .. }));
}

#[test]
fn raw_pointer_kinds_never_unify() {
    let (fixture, _, _) = check_errors("func f(p: *raw i32) -> *raw mut i32 { p }");
    assert!(
        fixture.codes().iter().any(|code| code == "E3001"),
        "expected a type mismatch, got {:?}",
        fixture.codes()
    );

    let (fixture, _, _) = check_errors("func f(p: *i32) -> *raw i32 { p }");
    assert!(
        fixture.codes().iter().any(|code| code == "E3001"),
        "a reference must not unify with a raw pointer, got {:?}",
        fixture.codes()
    );
}

#[test]
fn raw_pointers_are_accepted_in_fields_and_generics() {
    let (fixture, _, _) = check_clean(
        "struct Holder { pointer: *raw i32 }\n\
         func identity<T>(value: *raw T) -> *raw T { value }",
    );
    let _ = fixture;
}

#[test]
fn raw_remains_an_ordinary_identifier() {
    let (fixture, hir, results) = check_clean("func f(raw: i32) -> i32 { raw }");
    let (_, body) = function(&fixture, &hir, "f");
    let param = local_ty(&results, body.params[0].def);
    expect_ty(&hir, param, "i32");
}
