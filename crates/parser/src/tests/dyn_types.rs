//! `dyn Trait` type syntax and recovery.

use seal_ast::{PrimitiveType, TypeKind};

use super::support::{Fixture, assert_invariants, function, parse_clean, sole_function};

#[test]
fn dyn_parses_behind_shared_and_exclusive_pointers() {
    let (fixture, ast) = parse_clean("func f(a: *dyn Shape) -> *mut dyn Shape { a }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Pointer {
        pointee,
        mutable: false,
    } = ast.ty(parameter.ty).kind
    else {
        panic!("expected a shared pointer parameter");
    };
    let TypeKind::Dyn(path) = &ast.ty(pointee).kind else {
        panic!("expected a dyn pointee");
    };
    assert_eq!(fixture.symbol(path.name), "Shape");
    assert_eq!(fixture.text(path.name_span), "Shape");
    assert!(path.args.is_none());
    assert_eq!(fixture.text(ast.ty(pointee).span), "dyn Shape");

    let TypeKind::Pointer {
        pointee,
        mutable: true,
    } = ast.ty(declaration.ret.expect("return type")).kind
    else {
        panic!("expected an exclusive pointer return type");
    };
    let TypeKind::Dyn(path) = &ast.ty(pointee).kind else {
        panic!("expected a dyn return pointee");
    };
    assert_eq!(fixture.symbol(path.name), "Shape");
}

#[test]
fn dyn_accepts_generic_trait_arguments() {
    let (fixture, ast) = parse_clean("func f(a: *dyn Convert<i32>) {}");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Pointer { pointee, .. } = ast.ty(parameter.ty).kind else {
        panic!("expected a pointer parameter");
    };
    let TypeKind::Dyn(path) = &ast.ty(pointee).kind else {
        panic!("expected a dyn pointee");
    };
    assert_eq!(fixture.symbol(path.name), "Convert");
    let args = path.args.as_ref().expect("trait arguments");
    assert_eq!(args.args.len(), 1);
    assert!(matches!(
        ast.ty(args.args[0]).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert_eq!(fixture.text(args.span), "<i32>");
}

#[test]
fn dyn_parses_in_tuples_arrays_slices_and_nested_types() {
    let (fixture, ast) = parse_clean("func f(a: (dyn Shape, [*dyn Shape; 2], [dyn Shape])) {}");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Tuple(elements) = &ast.ty(parameter.ty).kind else {
        panic!("expected a tuple parameter");
    };
    assert!(matches!(ast.ty(elements[0]).kind, TypeKind::Dyn(_)));

    let TypeKind::Array { element, .. } = ast.ty(elements[1]).kind else {
        panic!("expected an array element type");
    };
    let TypeKind::Pointer { pointee, .. } = ast.ty(element).kind else {
        panic!("expected a pointer array element");
    };
    assert!(matches!(ast.ty(pointee).kind, TypeKind::Dyn(_)));

    let TypeKind::Slice { element } = ast.ty(elements[2]).kind else {
        panic!("expected a slice element type");
    };
    assert!(matches!(ast.ty(element).kind, TypeKind::Dyn(_)));
}

#[test]
fn dyn_parses_as_a_slice_element_in_a_return_type() {
    let (fixture, ast) = parse_clean("func f(a: [dyn Shape]) -> [dyn Shape] { a }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Slice { element } = ast.ty(parameter.ty).kind else {
        panic!("expected a slice parameter");
    };
    assert!(matches!(ast.ty(element).kind, TypeKind::Dyn(_)));
    let TypeKind::Slice { element } = ast.ty(declaration.ret.expect("return type")).kind else {
        panic!("expected a slice return type");
    };
    assert!(matches!(ast.ty(element).kind, TypeKind::Dyn(_)));
}

#[test]
fn dyn_without_a_trait_name_reports_one_error_and_keeps_the_delimiter() {
    let fixture = Fixture::new("func f(a: dyn) {}");
    let ast = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "expected a trait name after `dyn`");

    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Dyn(path) = &ast.ty(parameter.ty).kind else {
        panic!("expected a dyn error node");
    };
    assert!(fixture.symbol(path.name).is_empty());
}

#[test]
fn dyn_recovery_keeps_following_items() {
    let fixture = Fixture::new("func f(a: *dyn) {} func g() {}");
    let ast = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    assert_eq!(ast.program().items().len(), 2);
    assert_eq!(fixture.symbol(function(&ast, 1).name), "g");
}

#[test]
fn dyn_is_not_an_identifier() {
    for source in [
        "func dyn() {}",
        "func f() { let dyn = 1; }",
        "struct S { dyn: i32 }",
    ] {
        let fixture = Fixture::new(source);
        let _ = fixture.parse();
        assert!(
            !fixture.diagnostics().is_empty(),
            "`dyn` was accepted as a name in: {source}"
        );
    }
}

#[test]
fn dyn_without_a_name_in_a_return_type_recovers_at_the_body() {
    let fixture = Fixture::new("func f() -> dyn {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "expected a trait name after `dyn`");
}

#[test]
fn dyn_with_a_non_identifier_name_recovers_without_cascading() {
    let fixture = Fixture::new("func f(a: dyn 1) {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "expected a trait name after `dyn`");
}

#[test]
fn dyn_rejects_multiple_principal_traits_without_cascading() {
    let fixture = Fixture::new("func f(a: *dyn Shape + Other) {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "takes exactly one trait");
}

#[test]
fn dyn_rejects_a_trailing_plus() {
    let fixture = Fixture::new("func f(a: *dyn Shape +) {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "takes exactly one trait");
}

#[test]
fn dyn_reports_a_missing_generic_close() {
    let fixture = Fixture::new("func f(a: *dyn Convert<i32) {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "expected `,` or `>` in the type argument list");
}

#[test]
fn dyn_is_a_keyword_in_type_position_but_not_a_trait_name() {
    let fixture = Fixture::new("func f(a: dyn dyn) {}");
    let _ = fixture.parse();
    assert_eq!(
        fixture.diagnostics().len(),
        1,
        "unexpected diagnostics: {:?}",
        fixture.codes()
    );
    fixture.expect("E1002", "expected a trait name after `dyn`");
}
