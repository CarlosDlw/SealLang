//! `extern` blocks, functions, and statics.

use seal_ast::ItemKind;

use crate::tests::support::{Fixture, parse_clean};

#[test]
fn an_extern_block_keeps_its_declarations_in_order() {
    let source = r#"extern "c" {
        func abs(value: i32) -> i32;
        func getpid() -> i32 link_name "getpid";
        static mut errno: *raw mut i32;
    }"#;
    let (fixture, ast) = parse_clean(source);
    let item = ast.item(ast.program().items()[0]);
    let ItemKind::ExternBlock(block) = &item.kind else {
        panic!("expected an extern block, got {:?}", item.kind);
    };
    assert_eq!(fixture.symbol(block.abi), "c");
    assert_eq!(block.items.len(), 3);

    let ItemKind::ExternFunc(abs) = &ast.item(block.items[0]).kind else {
        panic!("expected an external function");
    };
    assert_eq!(fixture.symbol(abs.name), "abs");
    assert_eq!(abs.params.len(), 1);
    assert!(abs.ret.is_some());
    assert!(abs.link_name.is_none());

    let ItemKind::ExternFunc(getpid) = &ast.item(block.items[1]).kind else {
        panic!("expected an external function");
    };
    let link_name = getpid.link_name.expect("getpid has a link name");
    assert_eq!(fixture.symbol(link_name.symbol), "getpid");

    let ItemKind::ExternStatic(errno) = &ast.item(block.items[2]).kind else {
        panic!("expected an external static");
    };
    assert_eq!(fixture.symbol(errno.name), "errno");
    assert!(errno.mutable);
}

#[test]
fn an_extern_declaration_can_stand_alone() {
    let (fixture, ast) = parse_clean(r#"extern "c" func abs(value: i32) -> i32;"#);
    let ItemKind::ExternFunc(declaration) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an external function");
    };
    assert_eq!(fixture.symbol(declaration.name), "abs");
    assert_eq!(fixture.symbol(declaration.abi), "c");
}

#[test]
fn a_missing_return_type_is_void() {
    let (_, ast) = parse_clean(r#"extern "c" { func free(pointer: *raw mut u8); }"#);
    let ItemKind::ExternBlock(block) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an extern block");
    };
    let ItemKind::ExternFunc(declaration) = &ast.item(block.items[0]).kind else {
        panic!("expected an external function");
    };
    assert!(declaration.ret.is_none());
    assert!(!declaration.variadic);
}

#[test]
fn variadic_declarations_are_rejected() {
    let fixture = Fixture::new(r#"extern "c" { func printf(format: *raw u8, ...); }"#);
    let _ = fixture.parse();
    fixture.expect("E3088", "variadic");
}

#[test]
fn link_directives_name_libraries_in_source_order() {
    let (fixture, ast) =
        parse_clean(r#"extern "c" { link "m"; func abs(value: i32) -> i32; link "pthread"; }"#);
    let ItemKind::ExternBlock(block) = &ast.item(ast.program().items()[0]).kind else {
        panic!("expected an extern block");
    };
    assert_eq!(block.items.len(), 1);
    let names: Vec<String> = block
        .links
        .iter()
        .map(|link| fixture.symbol(link.name))
        .collect();
    assert_eq!(names, ["m", "pthread"]);
}

#[test]
fn a_malformed_link_directive_is_rejected() {
    let fixture = Fixture::new(r#"extern "c" { link m; }"#);
    let _ = fixture.parse();
    fixture.expect("E3091", "library name string");
}

#[test]
fn generic_external_functions_are_rejected() {
    let fixture = Fixture::new(r#"extern "c" { func identity<T>(value: T) -> T; }"#);
    let _ = fixture.parse();
    fixture.expect("E3093", "generic parameters");
}

#[test]
fn external_aggregates_are_rejected() {
    let fixture = Fixture::new(r#"extern "c" type opaque;"#);
    let _ = fixture.parse();
    fixture.expect("E3081", "external aggregate");
}

#[test]
fn an_external_struct_in_a_block_is_rejected() {
    let fixture = Fixture::new(r#"extern "c" { struct Point { x: i32 } }"#);
    let _ = fixture.parse();
    fixture.expect("E3081", "only contains declarations");
}

#[test]
fn a_body_on_an_external_function_is_rejected() {
    let fixture = Fixture::new(r#"extern "c" { func abs(value: i32) -> i32 { value } }"#);
    let _ = fixture.parse();
    fixture.expect("E3081", "ends with `;`");
}
