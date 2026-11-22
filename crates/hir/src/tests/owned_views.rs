//! HIR lowering for the `owned_view` builtin.

use crate::{ExprKind, Res};

use super::support::{function, lower_clean, tail};

#[test]
fn owned_view_lowers_to_its_own_expression_with_both_operands() {
    let (fixture, hir) = lower_clean(
        "struct Buffer { pointer: *raw i32 }\n\
         func f(buffer: *Buffer) -> *i32 {\n\
           unsafe { owned_view(*buffer, (*buffer).pointer) }\n\
         }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::Unsafe { block } = hir.expr(tail(&hir, body)).kind else {
        panic!("expected the unsafe block");
    };
    let view = hir.block(block).tail.expect("the unsafe block has a tail");
    let ExprKind::OwnedView { owner, source } = hir.expr(view).kind else {
        panic!("expected the owned_view builtin");
    };
    assert!(matches!(hir.expr(owner).kind, ExprKind::Deref { .. }));
    assert!(matches!(hir.expr(source).kind, ExprKind::Field { .. }));
}

#[test]
fn a_user_function_named_owned_view_shadows_the_builtin() {
    let (fixture, hir) = lower_clean(
        "struct Buffer { pointer: *raw i32 }\n\
         func owned_view(buffer: *Buffer, pointer: *raw i32) -> *i32 {\n\
           unsafe { &*pointer }\n\
         }\n\
         func f(buffer: *Buffer) -> *i32 { owned_view(buffer, (*buffer).pointer) }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let ExprKind::Call { callee, args } = &hir.expr(tail(&hir, body)).kind else {
        panic!("expected a normal call");
    };
    assert!(matches!(
        hir.expr(*callee).kind,
        ExprKind::Path {
            res: Res::Function(_),
            ..
        }
    ));
    assert_eq!(args.len(), 2);
}
