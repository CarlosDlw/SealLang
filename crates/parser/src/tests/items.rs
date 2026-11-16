//! Functions, parameters, and type syntax.

use seal_ast::{ItemKind, PrimitiveType, TypeKind};

use crate::tests::support::{Fixture, body, function, parse_clean, sole_function};

#[test]
fn functions_can_take_parameters_and_return_a_type() {
    let (fixture, ast) = parse_clean("func add(a: i32, b: *f64,) -> i32 { a }");
    let declaration = sole_function(&ast);
    assert_eq!(fixture.symbol(declaration.name), "add");
    assert_eq!(declaration.params.len(), 2);
    let first = ast.param(declaration.params[0]);
    assert_eq!(fixture.symbol(first.name), "a");
    assert!(matches!(
        ast.ty(first.ty).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    let second = ast.param(declaration.params[1]);
    assert_eq!(fixture.symbol(second.name), "b");
    match &ast.ty(second.ty).kind {
        TypeKind::Pointer(inner) => assert!(matches!(
            ast.ty(*inner).kind,
            TypeKind::Primitive(PrimitiveType::F64)
        )),
        other => panic!("expected pointer, got {other:?}"),
    }
    let ret = ast.ty(declaration.ret.expect("return type"));
    assert!(matches!(ret.kind, TypeKind::Primitive(PrimitiveType::I32)));
}

#[test]
fn functions_without_a_return_type_are_units() {
    let (_, ast) = parse_clean("func main() {}");
    let declaration = sole_function(&ast);
    assert!(declaration.ret.is_none());
}

#[test]
fn every_primitive_type_is_parsed() {
    let names = PrimitiveType::ALL.map(PrimitiveType::as_str);
    for name in names {
        let source = format!("func f(value: {name}) {{}}");
        let (_, ast) = parse_clean(&source);
        let declaration = sole_function(&ast);
        let param = ast.param(declaration.params[0]);
        match ast.ty(param.ty).kind {
            TypeKind::Primitive(primitive) => assert_eq!(primitive.as_str(), name),
            ref other => panic!("expected primitive {name}, got {other:?}"),
        }
    }
}

#[test]
fn pointers_can_nest() {
    let (_, ast) = parse_clean("func f(p: **i8) {}");
    let declaration = sole_function(&ast);
    let param = ast.param(declaration.params[0]);
    match &ast.ty(param.ty).kind {
        TypeKind::Pointer(outer) => match &ast.ty(*outer).kind {
            TypeKind::Pointer(inner) => assert!(matches!(
                ast.ty(*inner).kind,
                TypeKind::Primitive(PrimitiveType::I8)
            )),
            other => panic!("expected inner pointer, got {other:?}"),
        },
        other => panic!("expected pointer, got {other:?}"),
    }
}

#[test]
fn multiple_functions_keep_source_order() {
    let (fixture, ast) = parse_clean("func first() {} func second() -> i32 { 0 }");
    assert_eq!(ast.program().len(), 2);
    assert_eq!(fixture.symbol(function(&ast, 0).name), "first");
    assert_eq!(fixture.symbol(function(&ast, 1).name), "second");
    assert!(body(&ast, function(&ast, 1)).tail.is_some());
}

#[test]
fn unknown_types_report_e1004_and_keep_the_function() {
    let fixture = Fixture::new("func f(value: Missing) {}");
    let ast = fixture.parse();
    fixture.expect("E1004", "unknown type");
    let declaration = sole_function(&ast);
    let param = ast.param(declaration.params[0]);
    assert!(matches!(ast.ty(param.ty).kind, TypeKind::Error));
}

#[test]
fn missing_types_report_e1004() {
    let fixture = Fixture::new("func f(value:) {}");
    let ast = fixture.parse();
    fixture.expect("E1004", "expected a type");
    let declaration = sole_function(&ast);
    let param = ast.param(declaration.params[0]);
    assert!(matches!(ast.ty(param.ty).kind, TypeKind::Error));
}

#[test]
fn missing_function_names_report_e1005() {
    let fixture = Fixture::new("func () {}");
    let ast = fixture.parse();
    fixture.expect("E1005", "function name");
    assert_eq!(ast.program().len(), 1);
}

#[test]
fn missing_function_bodies_are_diagnosed() {
    let fixture = Fixture::new("func main()");
    let ast = fixture.parse();
    fixture.expect("E1002", "function body");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert!(block.statements.is_empty());
    assert!(block.tail.is_none());
}

#[test]
fn missing_parentheses_are_diagnosed() {
    let fixture = Fixture::new("func main(");
    let _ = fixture.parse();
    fixture.expect("E1002", "`)`");

    let fixture = Fixture::new("func f) {}");
    let _ = fixture.parse();
    fixture.expect("E1002", "`(`");
}

#[test]
fn a_missing_parameter_comma_is_diagnosed() {
    let fixture = Fixture::new("func f(a: i32 b: i32) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "between parameters");
    let declaration = sole_function(&ast);
    assert_eq!(declaration.params.len(), 2);
}

#[test]
fn top_level_items_are_functions_only() {
    let fixture = Fixture::new("let x = 1; func main() {}");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected an item");
    assert_eq!(ast.program().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[0]).kind,
        ItemKind::Error
    ));
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn empty_files_have_no_items() {
    let fixture = Fixture::new("");
    let ast = fixture.parse();
    fixture.assert_clean();
    assert!(ast.program().is_empty());
}
