//! End-to-end type checking of the example programs.

use std::path::PathBuf;

use super::support::{check_clean, expect_ty, function, let_binding, local_ty, statements};

const EXAMPLES: [&str; 6] = [
    "0001_main.sl",
    "0002_variables.sl",
    "0003_operators.sl",
    "004_conditionals.sl",
    "005_while_loop.sl",
    "006_structs.sl",
];

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn all_examples_type_check() {
    for name in EXAMPLES {
        let (fixture, hir, results) = check_clean(&source(name));
        let (main, _) = function(&fixture, &hir, "main");
        let signature = results.signature(main).expect("main has a signature");
        expect_ty(&hir, signature.ret, "i32");
    }
}

#[test]
fn variables_example_keeps_its_annotations() {
    let (fixture, hir, results) = check_clean(&source("0002_variables.sl"));
    let (_, main) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, main)[3]);
    expect_ty(&hir, local_ty(&results, binding), "f32");
}

#[test]
fn conditionals_example_has_the_expected_signatures() {
    let (fixture, hir, results) = check_clean(&source("004_conditionals.sl"));
    let (classify, _) = function(&fixture, &hir, "classify");
    let signature = results
        .signature(classify)
        .expect("classify has a signature");
    assert_eq!(signature.params.len(), 1);
    expect_ty(&hir, signature.ret, "i32");

    let (guard, _) = function(&fixture, &hir, "guard");
    let signature = results.signature(guard).expect("guard has a signature");
    expect_ty(&hir, signature.ret, "i32");
}

#[test]
fn while_example_has_the_expected_signature() {
    let (fixture, hir, results) = check_clean(&source("005_while_loop.sl"));
    let (sum, _) = function(&fixture, &hir, "sum_odd_below");
    let signature = results.signature(sum).expect("sum has a signature");
    assert_eq!(signature.params.len(), 1);
    expect_ty(&hir, signature.ret, "i32");
}
