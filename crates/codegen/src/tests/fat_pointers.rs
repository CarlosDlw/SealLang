//! Construction of fat pointers from thin data pointers.

use super::support::{front_end, function_ir, lowered_module_ir};

#[test]
fn with_len_builds_the_fat_pointer_without_exposing_provenance() {
    let ir = lowered_module_ir(&front_end(
        "func build(pointer: *raw mut u8) -> *raw mut [u8] {\n\
             unsafe { pointer.with_len(3) }\n\
         }\n\
         func main() -> i32 { 0 }",
    ));
    let body = function_ir(&ir, "seal.build");
    assert!(
        body.contains("insertvalue"),
        "the slice pointer is assembled from its words:\n{body}"
    );
    assert!(
        !body.contains("ptrtoint") && !body.contains("inttoptr"),
        "provenance must not round-trip through an integer:\n{body}"
    );
}
