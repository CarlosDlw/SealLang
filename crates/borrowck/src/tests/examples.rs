//! End-to-end borrow checking of the example programs.

use std::path::PathBuf;

use super::support::check_clean;

const EXAMPLES: [&str; 32] = [
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
    "036_default_methods.sl",
];

fn source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn a_default_method_example_borrow_checks() {
    check_clean(
        "struct Circle { radius: i32 } \
         trait Shape { \
             func area(self: *Self) -> i32; \
             func twice(self: *Self) -> i32 { self.area() * 2 } \
         } \
         impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } } \
         func main() -> i32 { \
             let circle = Circle { radius: 3 }; \
             circle.twice() \
         }",
    );
}

#[test]
fn all_examples_borrow_check() {
    for name in EXAMPLES {
        check_clean(&source(name));
    }
}
