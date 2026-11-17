//! End-to-end MIR lowering of the example programs.

use std::path::PathBuf;

use crate::terminator::TerminatorKind;

use super::support::{function, lower_clean};

const EXAMPLES: [&str; 17] = [
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
