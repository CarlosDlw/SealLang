//! Module declarations and the contextual names they use.

use seal_ast::{ExprKind, ItemKind, Modifiers, TypeKind};

use crate::tests::support::{Fixture, assert_invariants, parse_clean, sole_function, tail};

#[test]
fn multi_segment_value_paths_keep_every_name() {
    for source in [
        "func f() { root::a::b::C }",
        "func f() { a::b::C }",
        "func f() { a::b::C::<i32> }",
    ] {
        let (fixture, ast) = parse_clean(source);
        let declaration = sole_function(&ast);
        let expression = tail(&ast, declaration);
        let ExprKind::PathMulti { segments, .. } = &expression.kind else {
            panic!(
                "expected a multi-segment path in {source}, got {:?}",
                expression.kind
            );
        };
        assert!(!segments.is_empty(), "source: {source}");
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn a_single_root_segment_keeps_the_module_path_shape() {
    for source in [
        "func f() { root::main() }",
        "func f() { root::Point { x: 1 } }",
    ] {
        let (fixture, ast) = parse_clean(source);
        let expression = tail(&ast, sole_function(&ast));
        let expression = match &expression.kind {
            ExprKind::Call { callee, .. } => ast.expr(*callee),
            _ => expression,
        };
        assert!(
            matches!(
                expression.kind,
                ExprKind::PathMulti { .. } | ExprKind::StructMulti { .. }
            ),
            "source: {source}, got {:?}",
            expression.kind
        );
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn one_qualified_segment_keeps_the_associated_name_shape() {
    let (_, ast) = parse_clean("func f() { Type::assoc }");
    let expression = tail(&ast, sole_function(&ast));
    assert!(matches!(expression.kind, ExprKind::PathQualified { .. }));
}

#[test]
fn a_qualified_struct_literal_keeps_its_segments_and_fields() {
    let (fixture, ast) = parse_clean("func f() { a::b::C { field: 1 } }");
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::StructMulti {
        segments, fields, ..
    } = &expression.kind
    else {
        panic!(
            "expected a qualified struct literal, got {:?}",
            expression.kind
        );
    };
    assert_eq!(segments.len(), 2);
    assert_eq!(fields.len(), 1);
    assert_eq!(fixture.symbol(fields[0].name), "field");
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_qualified_type_path_is_kept_behind_a_pointer() {
    let (fixture, ast) = parse_clean("func f(value: *a::b::C) {}");
    let declaration = sole_function(&ast);
    let parameter = ast.param(declaration.params[0]);
    let TypeKind::Pointer { pointee, .. } = &ast.ty(parameter.ty).kind else {
        panic!("expected a pointer type");
    };
    assert!(matches!(ast.ty(*pointee).kind, TypeKind::PathMulti { .. }));
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_multi_segment_path_can_be_called() {
    let (fixture, ast) = parse_clean("func f() { a::b::C(1) }");
    let expression = tail(&ast, sole_function(&ast));
    let ExprKind::Call { callee, args } = &expression.kind else {
        panic!("expected a call, got {:?}", expression.kind);
    };
    assert!(matches!(ast.expr(*callee).kind, ExprKind::PathMulti { .. }));
    assert_eq!(args.len(), 1);
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_single_segment_module_path_keeps_its_name_and_span() {
    let (fixture, ast) = parse_clean("module geometry;");
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::Module(declaration) = &item.kind else {
        panic!("expected a module item, got {:?}", item.kind);
    };
    assert_eq!(declaration.path.len(), 1);
    assert_eq!(fixture.symbol(declaration.path[0]), "geometry");
    assert_eq!(fixture.text(declaration.path_span), "geometry");
    assert_eq!(item.modifiers, Modifiers::none());
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_nested_module_path_keeps_every_segment_in_order() {
    let (fixture, ast) = parse_clean("module geometry::shapes::polygons;");
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::Module(declaration) = &item.kind else {
        panic!("expected a module item, got {:?}", item.kind);
    };
    let names: Vec<String> = declaration
        .path
        .iter()
        .map(|name| fixture.symbol(*name))
        .collect();
    assert_eq!(names, ["geometry", "shapes", "polygons"]);
    assert_eq!(
        fixture.text(declaration.path_span),
        "geometry::shapes::polygons"
    );
}

#[test]
fn a_module_must_precede_imports_and_items() {
    let fixture = Fixture::new("func first() {} module late; func second() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2069"]);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Module(_)
    ));
    assert!(matches!(
        ast.item(ast.program().items()[2]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn a_module_after_an_import_is_rejected() {
    let fixture = Fixture::new("use root::C; module late;");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2069"]);
    fixture.expect("E2069", "must be the first item");
}

#[test]
fn a_file_declares_its_module_at_most_once() {
    let fixture = Fixture::new("module first; module second;");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2069"]);
    let items = ast.program().items();
    assert_eq!(items.len(), 2);
    assert!(matches!(ast.item(items[1]).kind, ItemKind::Module(_)));
}

#[test]
fn an_empty_module_path_is_an_error_but_the_file_continues() {
    let fixture = Fixture::new("module ; func next() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E2069"]);
    assert_eq!(ast.program().items().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn a_missing_module_semicolon_recovers_at_the_next_item() {
    let fixture = Fixture::new("module geometry func next() {}");
    let ast = fixture.parse();
    assert_eq!(fixture.codes(), ["E1002"]);
    assert_eq!(ast.program().items().len(), 2);
    assert!(matches!(
        ast.item(ast.program().items()[1]).kind,
        ItemKind::Func(_)
    ));
}

#[test]
fn a_cfg_module_is_a_conditional_module() {
    let fixture = Fixture::new("cfg(feature(\"std\")) module a;");
    let ast = fixture.parse();
    assert!(fixture.codes().is_empty());
    let item = ast.item(ast.program().items()[0]);
    assert!(item.modifiers.cfg.is_some());
}

#[test]
fn module_modifiers_are_rejected_with_one_diagnostic() {
    for source in [
        "public module a;",
        "internal module a;",
        "private module a;",
        "test module a;",
    ] {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_eq!(fixture.codes(), ["E2079"], "source: {source}");
        assert_eq!(ast.program().items().len(), 1);
        assert_invariants(&fixture, &ast);
    }
}

#[test]
fn root_cannot_name_a_module() {
    for source in ["module root;", "module root::nested;"] {
        let fixture = Fixture::new(source);
        fixture.parse();
        assert_eq!(fixture.codes(), ["E2075"], "source: {source}");
    }
}

#[test]
fn contextual_module_names_remain_valid_identifiers() {
    let sources = [
        "func module() {}",
        "func use() {}",
        "func public() {}",
        "func internal() {}",
        "func private() {}",
        "func cfg() {}",
        "func test() {}",
        "func root() {}",
        "func f(module: i32, use: i32, public: i32, root: i32) {}",
        "func f(value: module) {}",
        "func f() { let module = 1; let use = module; let public = use; }",
    ];
    for source in sources {
        let (fixture, ast) = parse_clean(source);
        assert_invariants(&fixture, &ast);
    }
}
