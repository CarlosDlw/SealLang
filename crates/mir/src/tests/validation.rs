//! Structural validation of MIR bodies.

use crate::body::{Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::place::Place;
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;
use crate::validate::ValidationError;

use super::support::{function, lower_clean};

fn main_body(source: &str) -> Body {
    let (fixture, hir, mir) = lower_clean(source);
    function(&fixture, &hir, &mir, "main").clone()
}

#[test]
fn bodies_without_blocks_are_rejected() {
    let mut body = main_body("func main() {}\n");
    body.blocks.clear();
    assert_eq!(body.validate(), Err(ValidationError::NoBlocks));
}

#[test]
fn entries_must_name_a_block() {
    let mut body = main_body("func main() {}\n");
    body.entry = BasicBlockId::from_u32(42).expect("42 is representable");
    assert!(matches!(
        body.validate(),
        Err(ValidationError::EntryOutOfRange { .. })
    ));
}

#[test]
fn bodies_need_a_return_local() {
    let mut body = main_body("func main() {}\n");
    body.locals.clear();
    assert_eq!(body.validate(), Err(ValidationError::MissingReturnLocal));
}

#[test]
fn local_zero_must_be_the_return_place() {
    let mut body = main_body("func main() {}\n");
    body.locals[Body::return_local()].kind = LocalKind::Binding;
    assert_eq!(
        body.validate(),
        Err(ValidationError::ReturnLocalHasWrongKind {
            kind: LocalKind::Binding
        })
    );
}

#[test]
fn parameters_must_be_argument_locals() {
    let mut body = main_body("func main(value: i32) -> i32 {\n  value\n}\n");
    body.locals[LocalId::from_u32(1).expect("one is representable")].kind = LocalKind::Temp;
    assert!(matches!(
        body.validate(),
        Err(ValidationError::ArgLocalHasWrongKind { .. })
    ));
}

#[test]
fn places_must_name_an_existing_local() {
    let mut body = main_body("func main() -> i32 {\n  0\n}\n");
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .next()
        .expect("the body has one assignment");
    let StatementKind::Assign { place, .. } = &mut statement.kind;
    *place = Place::local(LocalId::from_u32(99).expect("99 is representable"));

    assert!(matches!(
        body.validate(),
        Err(ValidationError::PlaceOutOfRange { .. })
    ));
}

#[test]
fn edges_must_name_an_existing_block() {
    let mut body = main_body("func main() -> i32 {\n  return 0;\n}\n");
    let entry = body.entry;
    body.blocks[entry].terminator.kind =
        TerminatorKind::Goto(BasicBlockId::from_u32(99).expect("99 is representable"));

    assert!(matches!(
        body.validate(),
        Err(ValidationError::BlockTargetOutOfRange { .. })
    ));
}

#[test]
fn switch_targets_are_checked() {
    let mut body = main_body("func main() {\n  if true {\n  }\n}\n");
    let entry = body.entry;
    let TerminatorKind::SwitchInt { targets, .. } = &mut body.blocks[entry].terminator.kind else {
        panic!("the entry block ends in a switch");
    };
    targets[0].target = BasicBlockId::from_u32(99).expect("99 is representable");

    assert!(matches!(
        body.validate(),
        Err(ValidationError::BlockTargetOutOfRange { .. })
    ));
}
