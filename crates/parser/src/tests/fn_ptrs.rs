//! Seal function pointer type syntax.

use seal_ast::{PrimitiveType, TypeKind};

use super::support::{Fixture, assert_invariants, parse_clean, sole_function};

/// Returns the parsed type of the first parameter of the only function.
fn first_parameter_type(ast: &seal_ast::Ast) -> seal_ast::TypeId {
    let declaration = sole_function(ast);
    ast.param(declaration.params[0]).ty
}

#[test]
fn a_function_pointer_parses_its_parameters_and_result() {
    let (fixture, ast) = parse_clean("func f(callback: func(i32, bool) -> i32) {}");
    assert_invariants(&fixture, &ast);

    let ty = first_parameter_type(&ast);
    let TypeKind::FnPtr {
        params,
        ret,
        is_unsafe,
    } = &ast.ty(ty).kind
    else {
        panic!("expected a function pointer type");
    };
    assert!(!is_unsafe);
    assert_eq!(params.len(), 2);
    assert!(matches!(
        ast.ty(params[0]).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert!(matches!(
        ast.ty(params[1]).kind,
        TypeKind::Primitive(PrimitiveType::Bool)
    ));
    let ret = ret.expect("the pointer declares a result");
    assert!(matches!(
        ast.ty(ret).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert_eq!(fixture.text(ast.ty(ty).span), "func(i32, bool) -> i32");
}

#[test]
fn a_function_pointer_omits_its_result_for_unit() {
    let (fixture, ast) = parse_clean("func f(callback: func()) {}");
    assert_invariants(&fixture, &ast);

    let TypeKind::FnPtr { params, ret, .. } = &ast.ty(first_parameter_type(&ast)).kind else {
        panic!("expected a function pointer type");
    };
    assert!(params.is_empty());
    assert!(ret.is_none());
}

#[test]
fn an_unsafe_function_pointer_records_its_qualifier() {
    let (fixture, ast) = parse_clean("func f(callback: unsafe func(i32)) {}");
    assert_invariants(&fixture, &ast);

    let ty = first_parameter_type(&ast);
    let TypeKind::FnPtr { is_unsafe, .. } = &ast.ty(ty).kind else {
        panic!("expected a function pointer type");
    };
    assert!(is_unsafe);
    assert_eq!(fixture.text(ast.ty(ty).span), "unsafe func(i32)");
}

#[test]
fn function_pointers_nest_in_parameters_and_results() {
    let (fixture, ast) = parse_clean("func f(value: func(func(i32, bool)) -> func()) {}");
    assert_invariants(&fixture, &ast);

    let TypeKind::FnPtr { params, ret, .. } = &ast.ty(first_parameter_type(&ast)).kind else {
        panic!("expected a function pointer type");
    };
    assert_eq!(params.len(), 1);
    let TypeKind::FnPtr {
        params: inner_params,
        ..
    } = &ast.ty(params[0]).kind
    else {
        panic!("expected a nested function pointer parameter");
    };
    assert_eq!(inner_params.len(), 2);

    let ret = ret.expect("the outer pointer declares a result");
    assert!(matches!(ast.ty(ret).kind, TypeKind::FnPtr { .. }));
}

#[test]
fn function_pointers_appear_in_tuples_and_arrays() {
    let (fixture, ast) =
        parse_clean("func f(pair: (func(), func(i32) -> i32), table: [func(); 2]) {}");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let TypeKind::Tuple(elements) = &ast.ty(ast.param(declaration.params[0]).ty).kind else {
        panic!("expected a tuple type");
    };
    assert_eq!(elements.len(), 2);
    for element in elements {
        assert!(matches!(ast.ty(*element).kind, TypeKind::FnPtr { .. }));
    }

    let TypeKind::Array { element, .. } = &ast.ty(ast.param(declaration.params[1]).ty).kind else {
        panic!("expected an array type");
    };
    assert!(matches!(ast.ty(*element).kind, TypeKind::FnPtr { .. }));
}

#[test]
fn function_pointers_appear_in_struct_fields() {
    let (fixture, ast) = parse_clean("struct Callback { run: func(i32) -> i32 }");
    assert_invariants(&fixture, &ast);

    let seal_ast::ItemKind::Struct(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a struct declaration");
    };
    let field = ast.field(declaration.fields[0]);
    assert!(matches!(ast.ty(field.ty).kind, TypeKind::FnPtr { .. }));
}

#[test]
fn a_variadic_function_pointer_is_rejected() {
    let fixture = Fixture::new("func f(callback: func(i32, ...)) {}");
    let ast = fixture.parse();
    fixture.expect("E3099", "cannot be variadic");
    assert_eq!(fixture.codes(), ["E3099"], "one diagnostic, no cascade");

    let TypeKind::FnPtr { params, .. } = &ast.ty(first_parameter_type(&ast)).kind else {
        panic!("recovery still produces a function pointer type");
    };
    assert_eq!(params.len(), 1, "the fixed parameters are kept");
}

#[test]
fn a_c_function_signature_is_not_a_seal_value_type() {
    let fixture = Fixture::new("func f(callback: extern \"c\" func(i32)) {}");
    let ast = fixture.parse();
    fixture.expect("E3099", "not a Seal value type");
    assert_eq!(fixture.codes(), ["E3099"]);

    let declaration = sole_function(&ast);
    assert_eq!(declaration.params.len(), 1);
}

#[test]
fn the_c_function_pointer_type_is_unchanged() {
    let (fixture, ast) = parse_clean("func f(callback: *raw extern \"c\" func(i32) -> i32) {}");
    assert_invariants(&fixture, &ast);

    let TypeKind::RawPointer { pointee, .. } = &ast.ty(first_parameter_type(&ast)).kind else {
        panic!("expected a raw pointer type");
    };
    assert!(matches!(ast.ty(*pointee).kind, TypeKind::FnSig { .. }));
}
