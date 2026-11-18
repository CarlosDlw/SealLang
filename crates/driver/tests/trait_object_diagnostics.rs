//! Command-line regression fixtures for trait-object diagnostics.
//!
//! Each case pins the stable code, the primary and secondary source locations,
//! a message fragment, and exit status 1 for a program a user can write.
//! Vtable collection limits that need lowered state stay covered by the
//! monomorphization unit tests.

mod support;

use support::{SourceFile, reject};

const NON_OBJECT_SAFE: &str = "\
trait Bad {
  func consume(self: Self) -> i32;
}
func use(value: *dyn Bad) -> i32 { 0 }
func main() -> i32 { 0 }
";

const MISSING_IMPLEMENTATION: &str = "\
trait Shape { func area(self: *Self) -> i32; }
struct Circle { radius: i32 }
func upcast(circle: *Circle) -> *dyn Shape { circle }
func main() -> i32 { 0 }
";

const INVALID_COERCION: &str = "\
trait Shape { func area(self: *Self) -> i32; }
struct Circle { radius: i32 }
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }
func upcast(circle: *mut Circle) -> *dyn Shape { circle }
func main() -> i32 { 0 }
";

const BARE_TRAIT_OBJECT: &str = "\
trait Shape { func area(self: *Self) -> i32; }
func take(value: dyn Shape) { }
func main() -> i32 { 0 }
";

const EXCLUSIVE_THROUGH_SHARED: &str = "\
trait Counter { func add(self: *mut Self, amount: i32); }
struct Cell { value: i32 }
impl Counter for Cell { func add(self: *mut Cell, amount: i32) { (*self).value += amount; } }
func bump(counter: *dyn Counter) { counter.add(1); }
func main() -> i32 { 0 }
";

const UNKNOWN_METHOD: &str = "\
trait Shape { func area(self: *Self) -> i32; }
struct Circle { radius: i32 }
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }
func read(value: *dyn Shape) -> i32 { value.nope() }
func main() -> i32 { 0 }
";

const AMBIGUOUS_SUPERTRAITS: &str = "\
trait Left { func name(self: *Self) -> i32; }
trait Right { func name(self: *Self) -> i32; }
trait Both: Left + Right { }
struct Value { value: i32 }
impl Left for Value { func name(self: *Value) -> i32 { (*self).value } }
impl Right for Value { func name(self: *Value) -> i32 { (*self).value } }
impl Both for Value { }
func read(value: *dyn Both) -> i32 { value.name() }
func main() -> i32 { 0 }
";

/// Requires `source` to be rejected at `location` with `code`.
fn reject_at(name: &str, source: &str, code: &str, location: &str, fragments: &[&str]) {
    let file = SourceFile::new(name, source);
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{name}: expected exit status 1, got {:?}: {stderr}",
        output.status
    );
    assert!(
        stderr.contains(&format!("error[{code}]")),
        "{name}: expected {code}, got: {stderr}"
    );
    assert!(
        stderr.contains(&format!(":{location}")),
        "{name}: expected the primary location {location}, got: {stderr}"
    );
    for fragment in fragments {
        assert!(
            stderr.contains(fragment),
            "{name}: expected {fragment:?} in: {stderr}"
        );
    }
}

#[test]
fn a_non_object_safe_trait_names_the_offending_method() {
    reject_at(
        "trait-object-non-object-safe",
        NON_OBJECT_SAFE,
        "E2065",
        "4:18",
        &[
            "the trait `Bad` cannot be used as a trait object",
            "method `consume` takes `self` by value",
            ":2:8",
        ],
    );
}

#[test]
fn a_concrete_pointer_without_the_impl_cannot_coerce() {
    reject_at(
        "trait-object-missing-impl",
        MISSING_IMPLEMENTATION,
        "E3046",
        "3:46",
        &[
            "`Circle` does not implement `dyn Shape`",
            "implement the trait for the concrete type",
        ],
    );
}

#[test]
fn a_mutable_pointer_does_not_coerce_to_a_shared_trait_object() {
    reject_at(
        "trait-object-reference-kind",
        INVALID_COERCION,
        "E3047",
        "4:50",
        &[
            "cannot coerce `*mut Circle` to `*dyn Shape`",
            "reference kinds differ",
            "reborrow the pointer explicitly",
        ],
    );
}

#[test]
fn a_trait_object_stored_by_value_names_the_reference_form() {
    reject_at(
        "trait-object-bare",
        BARE_TRAIT_OBJECT,
        "E3010",
        "2:11",
        &[
            "parameter cannot have the unsized type `dyn Shape`",
            "`*dyn Trait`",
        ],
    );
}

#[test]
fn an_exclusive_method_through_a_shared_object_names_both_spans() {
    reject_at(
        "trait-object-exclusive-shared",
        EXCLUSIVE_THROUGH_SHARED,
        "E3029",
        "4:44",
        &[
            "method `add` needs exclusive access through the trait object",
            "shared trait object",
            "`*mut dyn Trait`",
            ":4:36",
        ],
    );
}

#[test]
fn an_unknown_method_on_a_trait_object_is_rejected() {
    reject(
        "trait-object-unknown-method",
        UNKNOWN_METHOD,
        "E3027",
        "no method named `nope` is available for `dyn Shape`",
    );
}

#[test]
fn a_method_declared_by_two_supertraits_is_ambiguous_on_the_object() {
    reject_at(
        "trait-object-ambiguous-supertraits",
        AMBIGUOUS_SUPERTRAITS,
        "E2028",
        "8:44",
        &[
            "the method `name` on `dyn Both` is declared by more than one trait",
            "the traits are `Left`, `Right`",
        ],
    );
}
