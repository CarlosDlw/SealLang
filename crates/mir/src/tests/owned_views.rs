//! MIR lowering and structural validation for `owned_view`.

use seal_index::Idx;
use seal_ty::{RefKind, UintTy};

use crate::operand::{Constant, Operand};
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;
use crate::validate::ValidationError;

use super::support::{function, lower_clean, ty_text};

const THREE_FORMS: &str = "\
struct Thin { pointer: *raw i32 }
func thin(buffer: *Thin) -> *i32 { unsafe { owned_view(*buffer, (*buffer).pointer) } }
struct Slice { pointer: *raw [i32] }
func slice(buffer: *Slice) -> *[i32] { unsafe { owned_view(*buffer, (*buffer).pointer) } }
struct Text { pointer: *raw str }
func text(buffer: *Text) -> *str { unsafe { owned_view(*buffer, (*buffer).pointer) } }
";

fn owned_view_mut(body: &mut crate::body::Body) -> &mut Rvalue {
    body.blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find_map(|statement| match &mut statement.kind {
            StatementKind::Assign {
                rvalue: rvalue @ Rvalue::OwnedView { .. },
                ..
            } => Some(rvalue),
            _ => None,
        })
        .expect("the body builds an owned view")
}

fn owned_view(body: &crate::body::Body) -> Option<(&Rvalue, usize)> {
    for (index, block) in body.blocks.iter().enumerate() {
        for statement in &block.statements {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                continue;
            };
            if matches!(rvalue, Rvalue::OwnedView { .. }) {
                return Some((rvalue, index));
            }
        }
    }
    None
}

#[test]
fn every_owned_view_form_preserves_its_pointer_representation() {
    let (fixture, hir, mir) = lower_clean(THREE_FORMS);
    for (name, expected) in [("thin", "*i32"), ("slice", "*[i32]"), ("text", "*str")] {
        let body = function(&fixture, &hir, &mir, name);
        let Some((
            Rvalue::OwnedView {
                source, ty, kind, ..
            },
            _,
        )) = owned_view(body)
        else {
            panic!("`{name}` did not lower to an owned view");
        };
        assert_eq!(ty_text(&hir, *ty), expected);
        assert!(matches!(kind, RefKind::Shared));
        assert!(
            source.place().is_some(),
            "the source is a raw pointer value"
        );
    }
}

#[test]
fn mutable_owned_views_keep_the_exclusive_kind() {
    let source = "\
struct Buffer { pointer: *raw mut i32 }
func view(buffer: *mut Buffer) -> *mut i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "view");
    let Some((Rvalue::OwnedView { kind, ty, .. }, _)) = owned_view(body) else {
        panic!("the function did not lower to an owned view");
    };
    assert!(matches!(kind, RefKind::Exclusive));
    assert_eq!(ty_text(&hir, *ty), "*mut i32");
}

#[test]
fn an_owned_view_source_must_be_a_raw_pointer() {
    let source = "\
struct Buffer { pointer: *raw i32 }
func main(buffer: *Buffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = owned_view(&body).expect("the body builds a view").1;
    let block = crate::id::BasicBlockId::from_usize(block);
    let Rvalue::OwnedView { source, .. } = owned_view_mut(&mut body) else {
        unreachable!();
    };
    *source = Operand::Constant(Constant::Int {
        value: 0,
        ty: hir.ty_ctxt().uint_ty(UintTy::Usize),
    });
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::OwnedViewSourceNotRaw { block })
    );
}

#[test]
fn an_owned_view_owner_cannot_be_reached_through_a_raw_pointer() {
    let source = "\
struct Buffer { pointer: *raw i32 }
func main(buffer: *Buffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = owned_view(&body).expect("the body builds a view").1;
    let block = crate::id::BasicBlockId::from_usize(block);
    let Rvalue::OwnedView { owner, .. } = owned_view_mut(&mut body) else {
        unreachable!();
    };
    *owner = owner.clone().raw_deref();
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::OwnedViewRawDerefOwner { block })
    );
}

#[test]
fn an_owned_view_source_and_target_must_agree() {
    let source = "\
struct Buffer { pointer: *raw i32 }
func main(buffer: *Buffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = owned_view(&body).expect("the body builds a view").1;
    let block = crate::id::BasicBlockId::from_usize(block);
    {
        let Rvalue::OwnedView { kind, .. } = owned_view_mut(&mut body) else {
            unreachable!();
        };
        *kind = RefKind::Exclusive;
    }
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::OwnedViewKindMismatch { block })
    );

    {
        let Rvalue::OwnedView { kind, ty, .. } = owned_view_mut(&mut body) else {
            unreachable!();
        };
        *kind = RefKind::Shared;
        let byte = hir.ty_ctxt().uint_ty(UintTy::U8);
        *ty = hir.ty_ctxt().ref_ty_with(RefKind::Shared, byte);
    }
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::OwnedViewPointeeMismatch { block })
    );
}

#[test]
fn a_mutable_owned_view_keeps_the_raw_mutability() {
    let source = "\
struct Buffer { pointer: *raw mut i32 }
func main(buffer: *mut Buffer) -> *mut i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}";
    let (_fixture, _hir, mir) = lower_clean(source);
    assert_eq!(mir.bodies().len(), 1);
    let body = mir.bodies().iter().next().expect("one body");
    let Some((Rvalue::OwnedView { source, .. }, _)) = owned_view(body) else {
        panic!("the body builds a view");
    };
    let place = source.place().expect("the source is a place");
    assert!(matches!(
        place.projections(),
        [
            crate::place::Projection::Deref,
            crate::place::Projection::Field(_)
        ]
    ));
}
