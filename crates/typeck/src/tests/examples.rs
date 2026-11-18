//! End-to-end type checking of the example programs.

use std::path::PathBuf;

use super::support::{check_clean, expect_ty, function, let_binding, local_ty, statements};

const EXAMPLES: [&str; 31] = [
    "0001_main.sl",
    "0002_variables.sl",
    "0003_operators.sl",
    "004_conditionals.sl",
    "005_while_loop.sl",
    "006_structs.sl",
    "007_runtime_panic.sl",
    "008_exclusive_references.sl",
    "010_slices.sl",
    "011_methods.sl",
    "012_user_methods.sl",
    "013_method_borrows.sl",
    "014_method_namespaces.sl",
    "015_traits.sl",
    "016_trait_receivers.sl",
    "017_enums.sl",
    "018_tuple_variants.sl",
    "019_match.sl",
    "020_match_advanced.sl",
    "021_match_slice.sl",
    "022_match_reference.sl",
    "023_for.sl",
    "024_if_let.sl",
    "025_while_let.sl",
    "026_tuples.sl",
    "027_generic_functions.sl",
    "028_generic_structs.sl",
    "029_generic_enums.sl",
    "030_generic_traits.sl",
    "031_generic_recursion.sl",
    "035_numeric_casts.sl",
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
