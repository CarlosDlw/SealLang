//! Generic syntax: declarations, bounds, arguments, and recovery.

use seal_ast::{
    BoundKind, ExprKind, GenericArgs, ItemKind, StmtKind, TypeKind, TypePath, VariantFieldList,
};

use crate::tests::support::{Fixture, assert_invariants, body, parse_clean, sole_function};

fn function(ast: &seal_ast::Ast, index: usize) -> &seal_ast::FnDecl {
    let id = ast.program().items()[index];
    let ItemKind::Func(declaration) = &ast.item(id).kind else {
        panic!("item {index} is not a function");
    };
    declaration
}

fn struct_decl(ast: &seal_ast::Ast, index: usize) -> &seal_ast::StructDecl {
    let id = ast.program().items()[index];
    let ItemKind::Struct(declaration) = &ast.item(id).kind else {
        panic!("item {index} is not a struct");
    };
    declaration
}

fn path(ast: &seal_ast::Ast, ty: seal_ast::TypeId) -> &TypePath {
    match &ast.ty(ty).kind {
        TypeKind::Path(path) => path,
        other => panic!("expected a path type, got {other:?}"),
    }
}

fn args(ast: &seal_ast::Ast, ty: seal_ast::TypeId) -> &GenericArgs {
    path(ast, ty)
        .args
        .as_ref()
        .expect("the type has explicit arguments")
}

fn let_init(ast: &seal_ast::Ast, function: &seal_ast::FnDecl, index: usize) -> seal_ast::ExprId {
    let block = body(ast, function);
    let statement = ast.stmt(block.statements[index]);
    let StmtKind::Let(binding) = &statement.kind else {
        panic!("statement {index} is not a let");
    };
    binding.init.expect("the binding has an initializer")
}

#[test]
fn every_generic_item_kind_parses_with_parameters() {
    let (fixture, ast) = parse_clean(
        "struct Pair<T, U> { first: T, second: U } \
         enum Choice<T> { Some(T), None } \
         trait Convert<T, U, V> { func convert(self: *Self, value: T) -> U; } \
         impl<T, U> Pair<T, U> { \
             func first(self: *Pair<T, U>) -> T { (*self).first } \
         } \
         func id<T>(value: T) -> T { value }",
    );
    assert_invariants(&fixture, &ast);

    assert_eq!(struct_decl(&ast, 0).generics.len(), 2);
    let ItemKind::Enum(choice) = &ast.item(ast.program().items()[1]).kind else {
        panic!("expected an enum");
    };
    assert_eq!(choice.generics.len(), 1);
    let ItemKind::Trait(convert) = &ast.item(ast.program().items()[2]).kind else {
        panic!("expected a trait");
    };
    assert_eq!(convert.generics.len(), 3);
    let ItemKind::Impl(implementation) = &ast.item(ast.program().items()[3]).kind else {
        panic!("expected an impl");
    };
    assert_eq!(implementation.generics.len(), 2);
    assert!(implementation.trait_ref.is_none());
    assert_eq!(args(&ast, implementation.self_ty).args.len(), 2);
    assert_eq!(function(&ast, 4).generics.len(), 1);
}

#[test]
fn trailing_commas_are_accepted_in_every_generic_list() {
    let (fixture, ast) = parse_clean(
        "func f<T, U: Copy + Trait<V,>>(value: Pair<T, U,>) -> T \
         where T: Copy + Trait<U,>, U: Copy, { value }",
    );
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    assert_eq!(declaration.generics.len(), 2);
    assert_eq!(declaration.where_clause.len(), 2);
    let first = ast.generic_param(declaration.generics.params[0]);
    assert!(first.bounds.is_empty());
    let second = ast.generic_param(declaration.generics.params[1]);
    assert_eq!(second.bounds.len(), 2);
}

#[test]
fn inline_bounds_and_where_predicates_keep_their_kinds() {
    let (fixture, ast) = parse_clean(
        "func f<T: Copy + Trait<U>, U: Copy>(value: T) -> U \
         where T: Trait<U> + Copy, U: Copy, { value }",
    );
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let first = ast.generic_param(declaration.generics.params[0]);
    assert!(matches!(ast.bound(first.bounds[0]).kind, BoundKind::Copy));
    assert!(matches!(
        &ast.bound(first.bounds[1]).kind,
        BoundKind::Trait(trait_ref) if fixture.symbol(trait_ref.name) == "Trait"
    ));

    assert_eq!(declaration.where_clause.len(), 2);
    let predicate = &declaration.where_clause.predicates[0];
    assert_eq!(predicate.bounds.len(), 2);
    assert!(matches!(
        ast.bound(predicate.bounds[1]).kind,
        BoundKind::Copy
    ));
}

#[test]
fn nested_generic_types_split_shr_into_two_closers() {
    let (fixture, ast) = parse_clean(
        "struct Wrapper<T> { value: T } \
         func f(value: Wrapper<Wrapper<Wrapper<i32>>>) -> i32 { 0 }",
    );
    assert_invariants(&fixture, &ast);
    let declaration = function(&ast, 1);
    let parameter = ast.param(declaration.params[0]);
    let outer = args(&ast, parameter.ty);
    assert_eq!(outer.args.len(), 1);
    let middle = args(&ast, outer.args[0]);
    assert_eq!(middle.args.len(), 1);
    let inner = args(&ast, middle.args[0]);
    assert!(matches!(
        ast.ty(inner.args[0]).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));
}

#[test]
fn shifts_and_comparisons_stay_expressions_outside_generics() {
    let (_, ast) = parse_clean(
        "func shr(a: i32, b: i32) -> i32 { a >> b } \
         func shl(a: i32, b: i32) -> i32 { a << b } \
         func lt(a: i32, b: i32) -> bool { a < b } \
         func assign(a: i32, b: i32) -> i32 { a >>= b; a }",
    );

    let shr = function(&ast, 0);
    let tail = ast.expr(body(&ast, shr).tail.expect("tail"));
    assert!(matches!(
        tail.kind,
        ExprKind::Binary {
            op: seal_ast::BinaryOp::Shr,
            ..
        }
    ));

    let shl = function(&ast, 1);
    let tail = ast.expr(body(&ast, shl).tail.expect("tail"));
    assert!(matches!(
        tail.kind,
        ExprKind::Binary {
            op: seal_ast::BinaryOp::Shl,
            ..
        }
    ));

    let lt = function(&ast, 2);
    let tail = ast.expr(body(&ast, lt).tail.expect("tail"));
    assert!(matches!(
        tail.kind,
        ExprKind::Binary {
            op: seal_ast::BinaryOp::Lt,
            ..
        }
    ));

    let assign = function(&ast, 3);
    let statement = ast.stmt(body(&ast, assign).statements[0]);
    assert!(matches!(
        statement.kind,
        StmtKind::Expr(expr)
            if matches!(
                ast.expr(expr).kind,
                ExprKind::Assign {
                    op: seal_ast::AssignOp::ShrAssign,
                    ..
                }
            )
    ));
}

#[test]
fn turbofish_is_kept_on_every_expression_form() {
    let (fixture, ast) = parse_clean(
        "func f<T>() {} \
         func main() { \
             let a = f::<i32>(1); \
             let b = Make::<i32>::new(); \
             let c = Option::<i32>::Some(1); \
             let d = Pair::<i32, bool> { first: 1, second: true }; \
             let e = value.method::<i32>(); \
         }",
    );
    assert_invariants(&fixture, &ast);
    let main = function(&ast, 1);

    let a = let_init(&ast, main, 0);
    let ExprKind::Call { callee, .. } = &ast.expr(a).kind else {
        panic!("expected a call");
    };
    let ExprKind::Path {
        args: Some(args), ..
    } = &ast.expr(*callee).kind
    else {
        panic!("expected a generic path");
    };
    assert_eq!(args.args.len(), 1);

    let b = let_init(&ast, main, 1);
    let ExprKind::Call { callee, .. } = &ast.expr(b).kind else {
        panic!("expected a call");
    };
    let ExprKind::PathQualified {
        type_args: Some(args),
        name,
        ..
    } = &ast.expr(*callee).kind
    else {
        panic!("expected a generic qualified path");
    };
    assert_eq!(args.args.len(), 1);
    assert_eq!(fixture.symbol(*name), "new");

    let c = let_init(&ast, main, 2);
    let ExprKind::Call { callee, .. } = &ast.expr(c).kind else {
        panic!("expected a call");
    };
    assert!(matches!(
        &ast.expr(*callee).kind,
        ExprKind::PathQualified {
            type_args: Some(args),
            ..
        } if args.args.len() == 1
    ));

    let d = let_init(&ast, main, 3);
    assert!(matches!(
        &ast.expr(d).kind,
        ExprKind::Struct {
            type_args: Some(args),
            ..
        } if args.args.len() == 2
    ));

    let e = let_init(&ast, main, 4);
    assert!(matches!(
        &ast.expr(e).kind,
        ExprKind::MethodCall {
            generic_args: Some(args),
            ..
        } if args.args.len() == 1
    ));
}

#[test]
fn qualified_variant_literals_keep_type_arguments() {
    let (fixture, ast) = parse_clean(
        "enum Option<T> { Some(T), None } \
         func main() { let value = Option::<i32>::Some(1); }",
    );
    assert_invariants(&fixture, &ast);
    let main = function(&ast, 1);
    let value = let_init(&ast, main, 0);
    let ExprKind::Call { callee, .. } = &ast.expr(value).kind else {
        panic!("expected a call");
    };
    assert!(matches!(
        &ast.expr(*callee).kind,
        ExprKind::PathQualified {
            type_args: Some(args),
            name,
            ..
        } if args.args.len() == 1 && fixture.symbol(*name) == "Some"
    ));
}

#[test]
fn generic_types_nest_in_pointers_arrays_slices_and_tuples() {
    let (fixture, ast) = parse_clean(
        "func f(a: *Pair<T, U>, b: [Pair<T, U>; 2], c: [Pair<T, U>], \
         d: (Pair<T, U>, (T, U))) {}",
    );
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let parameters = &declaration.params;
    let pointer = ast.param(parameters[0]);
    let TypeKind::Pointer { pointee, .. } = ast.ty(pointer.ty).kind else {
        panic!("expected a pointer");
    };
    assert_eq!(args(&ast, pointee).args.len(), 2);

    let array = ast.param(parameters[1]);
    let TypeKind::Array { element, .. } = ast.ty(array.ty).kind else {
        panic!("expected an array");
    };
    assert_eq!(args(&ast, element).args.len(), 2);

    let slice = ast.param(parameters[2]);
    let TypeKind::Slice { element } = ast.ty(slice.ty).kind else {
        panic!("expected a slice");
    };
    assert_eq!(args(&ast, element).args.len(), 2);

    let tuple = ast.param(parameters[3]);
    let TypeKind::Tuple(elements) = &ast.ty(tuple.ty).kind else {
        panic!("expected a tuple");
    };
    assert_eq!(args(&ast, elements[0]).args.len(), 2);
}

#[test]
fn impl_headers_parse_inherent_trait_and_blanket_forms() {
    let (fixture, ast) = parse_clean(
        "impl<T> Pair<T> {} \
         impl<T, U> Pair<T, U> {} \
         impl<T> Trait<T> for Pair<T> {} \
         impl<T> Trait for T {} \
         impl Trait for i32 {} \
         impl<T> Trait<T> for i32 {}",
    );
    assert_invariants(&fixture, &ast);
    assert_eq!(ast.program().len(), 6);

    let ItemKind::Impl(first) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an impl");
    };
    assert_eq!(first.generics.len(), 1);
    assert!(first.trait_ref.is_none());
    assert_eq!(args(&ast, first.self_ty).args.len(), 1);

    let ItemKind::Impl(third) = &ast.item(ast.program().items()[2]).kind else {
        panic!("expected an impl");
    };
    let trait_ref = third.trait_ref.as_ref().expect("trait impl");
    assert_eq!(fixture.symbol(trait_ref.name), "Trait");
    assert_eq!(trait_ref.args.as_ref().expect("trait args").args.len(), 1);

    let ItemKind::Impl(blanket) = &ast.item(ast.program().items()[3]).kind else {
        panic!("expected an impl");
    };
    assert_eq!(fixture.symbol(path(&ast, blanket.self_ty).name), "T");

    let ItemKind::Impl(primitive) = &ast.item(ast.program().items()[4]).kind else {
        panic!("expected an impl");
    };
    assert!(matches!(
        ast.ty(primitive.self_ty).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::I32)
    ));
}

#[test]
fn self_is_a_type_path_inside_traits_and_impls() {
    let (fixture, ast) = parse_clean(
        "trait Convert<T> { func convert<U>(self: *Self, value: Pair<Self, T>) -> Self; } \
         struct S {} \
         impl<T> Convert<T> for S { \
             func convert<U>(self: *S, value: Pair<Self, T>) -> S { self } \
         }",
    );
    assert_invariants(&fixture, &ast);

    let ItemKind::Trait(convert) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected a trait");
    };
    let ItemKind::TraitMethod(method) = &ast.item(convert.methods[0]).kind else {
        panic!("expected a trait method");
    };
    assert_eq!(method.generics.len(), 1);
    let value = ast.param(method.params[1]);
    let type_args = args(&ast, value.ty);
    assert_eq!(fixture.symbol(path(&ast, type_args.args[0]).name), "Self");

    let ItemKind::Impl(implementation) = &ast.item(ast.program().items()[2]).kind else {
        panic!("expected an impl");
    };
    let ItemKind::Func(method) = &ast.item(implementation.functions[0]).kind else {
        panic!("expected an impl method");
    };
    assert_eq!(method.generics.len(), 1);
}

#[test]
fn where_clauses_are_recorded_before_bodies_semicolons_and_braces() {
    let (fixture, ast) = parse_clean(
        "struct Pair<T> where T: Copy { value: T } \
         trait Trait<T> where T: Copy { func f(self: *Self); } \
         impl<T> Pair<T> where T: Copy { func f(self: *Pair<T>) {} } \
         func f<T>(value: T) -> T where T: Copy { value }",
    );
    assert_invariants(&fixture, &ast);
    for item in ast.program().items() {
        let clause = match &ast.item(*item).kind {
            ItemKind::Struct(declaration) => &declaration.where_clause,
            ItemKind::Trait(declaration) => &declaration.where_clause,
            ItemKind::Impl(declaration) => &declaration.where_clause,
            ItemKind::Func(declaration) => &declaration.where_clause,
            _ => continue,
        };
        assert_eq!(clause.len(), 1);
        assert!(!clause.span.is_empty());
    }
}

#[test]
fn generic_spans_cover_each_delimiter_and_name() {
    let (fixture, ast) =
        parse_clean("func f<T: Copy>(value: Pair<i32, bool>) -> T where T: Copy { value }");
    let declaration = sole_function(&ast);
    assert_eq!(fixture.text(declaration.generics.span), "<T: Copy>");
    let parameter = ast.generic_param(declaration.generics.params[0]);
    assert_eq!(fixture.text(parameter.name_span), "T");
    assert_eq!(fixture.text(ast.bound(parameter.bounds[0]).span), "Copy");
    assert_eq!(fixture.text(declaration.where_clause.span), "where T: Copy");

    let value = ast.param(declaration.params[0]);
    let type_args = args(&ast, value.ty);
    assert_eq!(fixture.text(type_args.span), "<i32, bool>");
    assert_eq!(fixture.text(ast.ty(type_args.args[0]).span), "i32");
    assert_eq!(fixture.text(ast.ty(type_args.args[1]).span), "bool");
    assert!(
        fixture
            .text(ast.item(ast.program().items()[0]).span)
            .starts_with("func f")
    );
}

#[test]
fn empty_generic_parameter_and_argument_lists_are_rejected_once() {
    let fixture = Fixture::new("func f<>(x: i32) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "cannot be empty");
    assert!(sole_function(&ast).generics.is_empty());

    let fixture = Fixture::new("func f(value: Pair<>) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "cannot be empty");
    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let arguments = args(&ast, parameter.ty);
    assert!(arguments.args.is_empty());
}

#[test]
fn a_missing_generic_closer_recovers_at_the_next_item() {
    let fixture = Fixture::new("func f<T(x: T) {} func g() {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "close the generic parameter list");
    assert_eq!(ast.program().len(), 2);
    assert_eq!(function(&ast, 0).generics.len(), 1);
    assert_eq!(function(&ast, 1).params.len(), 0);
}

#[test]
fn missing_commas_in_parameters_and_arguments_do_not_cascade() {
    let fixture = Fixture::new("struct Pair<T U> { value: T }");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `,` or `>`");
    assert_eq!(struct_decl(&ast, 0).generics.len(), 2);
    assert_eq!(fixture.diagnostics().len(), 1);

    let fixture = Fixture::new("func f(value: Pair<i32 bool>) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "type argument list");
    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    assert_eq!(args(&ast, parameter.ty).args.len(), 2);
    assert_eq!(fixture.diagnostics().len(), 1);
}

#[test]
fn a_bound_without_a_name_is_rejected_once() {
    let fixture = Fixture::new("func f<T: + Copy>(value: T) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected a bound name");
    let declaration = sole_function(&ast);
    let parameter = ast.generic_param(declaration.generics.params[0]);
    assert_eq!(parameter.bounds.len(), 1);
    assert_eq!(fixture.diagnostics().len(), 1);
}

#[test]
fn an_empty_where_clause_is_rejected() {
    let fixture = Fixture::new("func f() where {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "needs at least one predicate");
    assert!(sole_function(&ast).where_clause.is_empty());
}

#[test]
fn method_arguments_without_turbofish_get_a_targeted_message() {
    let fixture = Fixture::new("func f() { value.m<u8>(); }");
    let ast = fixture.parse();
    fixture.expect("E1002", "require `::`");
    let declaration = sole_function(&ast);
    let statement = ast.stmt(body(&ast, declaration).statements[0]);
    let StmtKind::Expr(expr) = statement.kind else {
        panic!("expected an expression statement");
    };
    assert!(matches!(
        ast.expr(expr).kind,
        ExprKind::MethodCall {
            generic_args: Some(_),
            ..
        }
    ));
}

#[test]
fn malformed_generic_headers_recover_without_consuming_the_body() {
    let fixture = Fixture::new("impl<T Pair<T> { func f(self: *Pair<T>) {} }");
    let ast = fixture.parse();
    fixture.expect("E1002", "close the generic parameter list");
    assert_eq!(ast.program().len(), 1);

    let fixture = Fixture::new("impl<> Marker {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "cannot be empty");
    assert_eq!(ast.program().len(), 1);

    let fixture = Fixture::new("impl Trait for {}");
    let ast = fixture.parse();
    fixture.expect("E1004", "expected a type");
    assert_eq!(ast.program().len(), 1);
}

#[test]
fn unknown_parameter_names_in_bounds_are_left_for_hir() {
    let (fixture, ast) = parse_clean("func f<T: Trait<U>>(value: T) {}");
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let parameter = ast.generic_param(declaration.generics.params[0]);
    let BoundKind::Trait(trait_ref) = &ast.bound(parameter.bounds[0]).kind else {
        panic!("expected a trait bound");
    };
    assert_eq!(trait_ref.args.as_ref().expect("trait args").args.len(), 1);
}

#[test]
fn pattern_nodes_do_not_gain_type_arguments() {
    let (fixture, ast) = parse_clean(
        "enum Option<T> { Some(T), None } \
         func f(value: Option<i32>) -> i32 { match value { Option::Some(x) => x, Option::None => 0 } }",
    );
    assert_invariants(&fixture, &ast);
    let declaration = function(&ast, 1);
    let block = body(&ast, declaration);
    let tail = ast.expr(block.tail.expect("match tail"));
    let ExprKind::Match { arms, .. } = &tail.kind else {
        panic!("expected a match");
    };
    assert_eq!(arms.len(), 2);
}

#[test]
fn generic_tuple_variant_fields_keep_their_types() {
    let (fixture, ast) = parse_clean(
        "enum Pair<T, U> { Both(T, U) } \
         func f(value: Pair<i32, bool>) {}",
    );
    assert_invariants(&fixture, &ast);
    let ItemKind::Enum(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an enum");
    };
    let variant = ast.variant(declaration.variants[0]);
    let VariantFieldList::Tuple(fields) = &variant.fields else {
        panic!("expected positional fields");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(
        fixture.symbol(path(&ast, ast.tuple_field(fields[0]).ty).name),
        "T"
    );
}

#[test]
fn specified_recovery_cases_report_and_keep_the_tree_well_formed() {
    let fixture = Fixture::new("func f() { let x: Pair<i32 = 1; }");
    let ast = fixture.parse();
    fixture.expect("E1002", "close the type argument list");
    assert_invariants(&fixture, &ast);

    let fixture = Fixture::new("func f<T:>(x: T) {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected a bound after `:`");
    assert_invariants(&fixture, &ast);

    let fixture = Fixture::new("func f() { f::<>(); }");
    let ast = fixture.parse();
    fixture.expect("E1002", "generic argument list cannot be empty");
    assert_invariants(&fixture, &ast);

    let fixture = Fixture::new("func f(x: i32) { f::<Pair<i32, bool>>(x); }");
    let ast = fixture.parse();
    assert_invariants(&fixture, &ast);
    assert!(fixture.diagnostics().is_empty());

    let fixture = Fixture::new("func f() where T {}");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `:` after the where-clause type");
    assert_invariants(&fixture, &ast);
}

#[test]
fn generated_unclosed_generic_forms_recover_without_panicking() {
    for depth in 1..=8 {
        let params = "T, ".repeat(depth);
        let arguments = "i32, ".repeat(depth);
        for source in [
            format!("func f<{params}(value: i32) -> i32 {{ value }}"),
            format!("struct S<{params} {{ value: i32 }}"),
            format!("func f() {{ let value: Pair<{arguments} = 0; }}"),
        ] {
            let fixture = Fixture::new(&source);
            let ast = fixture.parse();
            assert_invariants(&fixture, &ast);
            assert!(
                !fixture.diagnostics().is_empty(),
                "an unclosed generic form must report a diagnostic: {source:?}"
            );
        }
    }
}

#[test]
fn generic_forms_terminate_on_hostile_combinations() {
    for source in [
        "func f<T",
        "func f<T,",
        "func f<T:",
        "func f<T: +",
        "func f(x: Pair<",
        "func f(x: Pair<i32,",
        "func f(x: Pair<Pair<i32, bool>) {}",
        "func f() { Pair::<i32, bool>> { first: 1 }; }",
        "func f() { value.m<Pair<u8, bool>>(); }",
        "impl<T Pair<T> {",
        "impl Trait for",
    ] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_invariants(&fixture, &ast);
    }
}
