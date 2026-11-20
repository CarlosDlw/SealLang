//! Fingerprints of definitions that share a name across modules.

use crate::instance_hash;
use crate::tests::support::collect_clean_package;

#[test]
fn same_named_functions_in_different_modules_hash_differently() {
    let (fixture, hir, _mir, program) = collect_clean_package(&[
        (
            "main.sl",
            "use alpha::run;\n\
             use beta::run as beta_run;\n\
             func main() -> i32 { run() + beta_run() }",
        ),
        (
            "alpha.sl",
            "module alpha;\ninternal func run() -> i32 { 1 }",
        ),
        ("beta.sl", "module beta;\ninternal func run() -> i32 { 2 }"),
    ]);

    let hashes: Vec<u128> = program
        .instances()
        .iter()
        .filter(|instance| fixture.symbol(hir.definitions().name(instance.instance.def)) == "run")
        .map(|instance| instance_hash(&hir, instance.instance).as_u128())
        .collect();
    assert_eq!(hashes.len(), 2, "both modules declare `run`");
    assert_ne!(hashes[0], hashes[1]);
}
