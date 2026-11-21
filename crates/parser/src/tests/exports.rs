//! Parsing of explicit C exports.

use crate::tests::support::{Fixture, assert_invariants, parse_clean};

#[test]
fn an_export_records_its_abi_and_name() {
    let (fixture, ast) =
        parse_clean("export \"c\" func add(a: i32, b: i32) -> i32 {\n  a + b\n}\n");
    let item = ast.item(ast.program().items()[0]);
    let abi = item.modifiers.export.expect("the export modifier exists");
    assert_eq!(fixture.interner.resolve(abi), "c");
    assert_invariants(&fixture, &ast);
}

#[test]
fn export_is_rejected_on_a_non_function_item() {
    let fixture = Fixture::new("export \"c\" struct Point { x: i32 }");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E1001"]);
}

#[test]
fn export_is_rejected_on_an_implementation_method() {
    let fixture = Fixture::new(
        "struct Point {}\n\
         impl Point { export \"c\" func x(self: *Point) -> i32 { 0 } }\n",
    );
    fixture.parse();
    assert_eq!(fixture.codes(), ["E1001"]);
}

#[test]
fn export_is_rejected_on_an_import() {
    let fixture = Fixture::new("export \"c\" use geometry::Circle;");
    fixture.parse();
    assert_eq!(fixture.codes(), ["E2079"]);
}
