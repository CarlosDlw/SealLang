//! Inherent `str` method calls in MIR.

use crate::operand::Operand;
use crate::rvalue::{CallTarget, Rvalue};

use super::support::{assignment, function, local_named, lower_clean_in_core};

#[test]
fn is_empty_lowers_to_a_direct_call_with_the_string_receiver() {
    let source = "\
        impl str { \
            public func is_empty(self: *str) -> bool { self.len() == 0 } \
        } \
        func main() -> bool { \
            let text: *str = \"hello\"; \
            text.is_empty() \
        }";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let body = function(&fixture, &hir, &mir, "main");
    let text = local_named(&fixture, body, "text");

    let (target, args) = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Call { target, args, .. } = rvalue else {
                return None;
            };
            let CallTarget::Direct { instance, .. } = target else {
                return None;
            };
            (fixture.symbol(hir.definitions().name(instance.def)) == "is_empty")
                .then_some((target, args.as_slice()))
        })
        .expect("is_empty must lower to a direct call");

    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(args.len(), 1, "the receiver is the only argument");
    let Operand::Copy(place) = &args[0] else {
        panic!("the string receiver must be passed as a copied fat pointer");
    };
    assert_eq!(place.local_id(), Some(text));
}
