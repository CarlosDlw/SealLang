//! C function pointer type syntax.

use seal_ast::{PrimitiveType, TypeKind};

use super::support::{Fixture, assert_invariants, parse_clean, sole_function};

#[test]
fn a_c_function_pointer_parses_its_abi_parameters_and_result() {
    let (fixture, ast) = parse_clean(
        r#"func f(callback: *raw extern "c" func(i32, *raw mut c_void, ...) -> c_int) {}"#,
    );
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::RawPointer {
        pointee,
        mutable: false,
    } = ast.ty(parameter.ty).kind
    else {
        panic!("expected a read-only raw function pointer");
    };
    let TypeKind::FnSig {
        abi,
        params,
        ret,
        variadic,
        ..
    } = &ast.ty(pointee).kind
    else {
        panic!("expected a C function signature");
    };
    assert_eq!(fixture.symbol(*abi), "c");
    assert_eq!(params.len(), 2);
    assert!(matches!(
        ast.ty(params[0]).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert!(matches!(
        ast.ty(params[1]).kind,
        TypeKind::RawPointer { .. }
    ));
    let ret = ret.expect("the signature declares a result");
    assert!(matches!(
        ast.ty(ret).kind,
        TypeKind::Primitive(PrimitiveType::CInt)
    ));
    assert!(*variadic);
}

#[test]
fn a_c_function_pointer_omits_its_result_for_void() {
    let (fixture, ast) = parse_clean(r#"func f(callback: *raw extern "c" func()) {}"#);
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::RawPointer { pointee, .. } = ast.ty(parameter.ty).kind else {
        panic!("expected a raw pointer");
    };
    let TypeKind::FnSig { params, ret, .. } = &ast.ty(pointee).kind else {
        panic!("expected a C function signature");
    };
    assert!(params.is_empty());
    assert!(ret.is_none());
}

#[test]
fn a_mutable_c_function_pointer_is_rejected_and_recovers_read_only() {
    let fixture = Fixture::new(r#"func f(callback: *raw mut extern "c" func()) {}"#);
    let ast = fixture.parse();
    fixture.expect("E3089", "cannot be `mut`");

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    assert!(matches!(
        ast.ty(parameter.ty).kind,
        TypeKind::RawPointer { mutable: false, .. }
    ));
}
