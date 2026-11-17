//! Inherent impl blocks and `Type::name` paths.

use seal_ast::{ExprKind, ItemKind, TypeKind};

use super::support::{body, parse_clean};

#[test]
fn impl_blocks_group_functions_under_a_type() {
    let (fixture, ast) = parse_clean(
        "struct Point { x: i32 } \
         impl Point { \
             func new(x: i32) -> Point { Point { x: x } } \
             func get(self: *Point) -> i32 { (*self).x } \
             func shift(self: *mut Point, delta: i32) { (*self).x += delta; } \
         }",
    );

    assert_eq!(ast.program().len(), 2);
    let impl_id = ast.program().items()[1];
    let ItemKind::Impl(declaration) = &ast.item(impl_id).kind else {
        panic!("expected an impl item");
    };
    assert_eq!(fixture.symbol(declaration.type_name), "Point");
    assert_eq!(declaration.functions.len(), 3);

    let names: Vec<String> = declaration
        .functions
        .iter()
        .map(|id| {
            let ItemKind::Func(function) = &ast.item(*id).kind else {
                panic!("impl members must be functions");
            };
            fixture.symbol(function.name)
        })
        .collect();
    assert_eq!(names, ["new", "get", "shift"]);

    let ItemKind::Func(get) = &ast.item(declaration.functions[1]).kind else {
        panic!("expected a function");
    };
    let receiver = ast.param(get.params[0]);
    assert_eq!(fixture.symbol(receiver.name), "self");
    assert!(matches!(
        ast.ty(receiver.ty).kind,
        TypeKind::Pointer { mutable: false, .. }
    ));

    let ItemKind::Func(shift) = &ast.item(declaration.functions[2]).kind else {
        panic!("expected a function");
    };
    let receiver = ast.param(shift.params[0]);
    assert!(matches!(
        ast.ty(receiver.ty).kind,
        TypeKind::Pointer { mutable: true, .. }
    ));
}

#[test]
fn multiple_impl_blocks_are_separate_items() {
    let (_, ast) = parse_clean(
        "struct Counter { value: i32 } \
         impl Counter { func zero() -> Counter { Counter { value: 0 } } } \
         impl Counter { func get(self: *Counter) -> i32 { (*self).value } }",
    );

    assert_eq!(ast.program().len(), 3);
    for item_id in ast.program().items().iter().skip(1) {
        assert!(matches!(ast.item(*item_id).kind, ItemKind::Impl(_)));
    }
}

#[test]
fn qualified_paths_are_parsed_as_associated_names() {
    let (fixture, ast) = parse_clean(
        "struct Point { x: i32 } \
         impl Point { func origin() -> Point { Point { x: 0 } } } \
         func main() -> Point { Point::origin() }",
    );
    let main = sole_function_at(&ast, 2);
    let ExprKind::Call { callee, .. } = &tail_of(&ast, main).kind else {
        panic!("expected a call");
    };
    match &ast.expr(*callee).kind {
        ExprKind::PathQualified {
            type_name, name, ..
        } => {
            assert_eq!(fixture.symbol(*type_name), "Point");
            assert_eq!(fixture.symbol(*name), "origin");
        }
        other => panic!("expected a qualified path, got {other:?}"),
    }
}

#[test]
fn malformed_impl_members_are_diagnosed() {
    let fixture = super::support::Fixture::new("struct Point { x: i32 } impl Point { let x = 1; }");
    let ast = fixture.parse();
    fixture.expect("E1001", "expected `func` in the impl block");
    let ItemKind::Impl(declaration) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected an impl item");
    };
    assert!(declaration.functions.is_empty());
}

#[test]
fn paths_without_a_name_are_diagnosed() {
    let fixture = super::support::Fixture::new(
        "struct Point { x: i32 } impl Point {} func main() -> i32 { Point:: }",
    );
    let ast = fixture.parse();
    fixture.expect("E1005", "associated name after `::`");
    let main = sole_function_at(&ast, 2);
    assert!(matches!(
        tail_of(&ast, main).kind,
        ExprKind::PathQualified { .. }
    ));
}

fn sole_function_at(ast: &seal_ast::Ast, index: usize) -> &seal_ast::FnDecl {
    let id = ast.program().items()[index];
    let ItemKind::Func(declaration) = &ast.item(id).kind else {
        panic!("item {index} is not a function");
    };
    declaration
}

fn tail_of<'a>(ast: &'a seal_ast::Ast, declaration: &seal_ast::FnDecl) -> &'a seal_ast::Expr {
    let block = body(ast, declaration);
    ast.expr(block.tail.expect("function has a tail"))
}

#[test]
fn impl_target_is_not_parsed_as_a_primitive() {
    let (fixture, ast) =
        parse_clean("struct Marker {} impl Marker { func make() -> Marker { Marker {} } }");
    let ItemKind::Impl(declaration) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected an impl item");
    };
    assert_eq!(fixture.symbol(declaration.type_name), "Marker");
    let ItemKind::Func(make) = &ast.item(declaration.functions[0]).kind else {
        panic!("expected a function");
    };
    assert!(matches!(
        ast.ty(make.ret.expect("return type")).kind,
        TypeKind::Path(_)
    ));
    assert!(make.params.is_empty());
}
