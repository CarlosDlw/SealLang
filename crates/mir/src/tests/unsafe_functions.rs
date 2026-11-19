//! Unsafe function calls stay ordinary MIR calls.

use crate::{Rvalue, StatementKind};

use super::support::{function, lower_clean};

#[test]
fn an_unsafe_call_lowers_to_the_ordinary_call_rvalue() {
    let (fixture, hir, mir) = lower_clean(
        "unsafe func danger(value: i32) -> i32 { unsafe { value } } \
         func main() -> i32 { unsafe { danger(1) } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let found = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| {
            matches!(
                &statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call { .. },
                    ..
                }
            )
        });
    assert!(found, "the unsafe marker does not create a new call shape");
}
