//! End-to-end MIR lowering of the example programs.

use std::path::PathBuf;

use crate::terminator::TerminatorKind;

use super::support::{function, lower_clean};

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
fn all_examples_lower_and_validate() {
    for name in EXAMPLES {
        let (fixture, hir, mir) = lower_clean(&source(name));
        function(&fixture, &hir, &mir, "main");
    }
}

#[test]
fn a_default_method_example_lowers_and_validates() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { \
            func area(self: *Self) -> i32; \
            func twice(self: *Self) -> i32 { self.area() * 2 } \
        } \
        impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } } \
        func main() -> i32 { \
            let circle = Circle { radius: 3 }; \
            circle.twice() \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    function(&fixture, &hir, &mir, "main");
}

#[test]
fn while_example_lowers_switches_and_back_edges() {
    let (fixture, hir, mir) = lower_clean(&source("005_while_loop.sl"));
    let body = function(&fixture, &hir, &mir, "sum_odd_below");

    assert!(body.blocks.len() > 4);
    assert!(
        body.blocks
            .iter()
            .any(|block| matches!(&block.terminator.kind, TerminatorKind::SwitchInt { .. }))
    );
    assert!(body.blocks.iter().enumerate().any(|(index, block)| {
        matches!(
            &block.terminator.kind,
            TerminatorKind::Goto(target) if target.index() as usize <= index
        )
    }));
}

#[test]
fn conditionals_example_lowers_nested_branches() {
    let (fixture, hir, mir) = lower_clean(&source("004_conditionals.sl"));
    let body = function(&fixture, &hir, &mir, "classify");

    assert!(body.blocks.len() >= 5);
    let switches = body
        .blocks
        .iter()
        .filter(|block| matches!(&block.terminator.kind, TerminatorKind::SwitchInt { .. }))
        .count();
    assert!(switches >= 2, "nested branches need several switches");
}
