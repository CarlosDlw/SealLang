//! Structural validation for MIR.

use std::fmt;

use crate::body::{Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::operand::Operand;
use crate::place::Place;
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;

/// A broken MIR invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// The body has no basic blocks.
    NoBlocks,
    /// The entry block is not in the body.
    EntryOutOfRange {
        /// The recorded entry.
        entry: BasicBlockId,
        /// The number of blocks.
        blocks: usize,
    },
    /// The body has no locals at all.
    MissingReturnLocal,
    /// Local zero is not the return place.
    ReturnLocalHasWrongKind {
        /// The kind found at local zero.
        kind: LocalKind,
    },
    /// The argument count does not leave room for the return local.
    ArgCountOutOfRange {
        /// The recorded argument count.
        arg_count: usize,
        /// The number of locals.
        locals: usize,
    },
    /// A parameter local has the wrong kind.
    ArgLocalHasWrongKind {
        /// The local.
        local: LocalId,
        /// The kind found.
        kind: LocalKind,
    },
    /// An argument-kind local sits outside the argument range.
    UnexpectedArg {
        /// The local.
        local: LocalId,
    },
    /// A place names a local that is not in the body.
    PlaceOutOfRange {
        /// The block containing the place.
        block: BasicBlockId,
        /// The invalid local.
        local: LocalId,
    },
    /// A control-flow edge names a block that is not in the body.
    BlockTargetOutOfRange {
        /// The block containing the edge.
        block: BasicBlockId,
        /// The invalid target.
        target: BasicBlockId,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBlocks => f.write_str("MIR body has no basic blocks"),
            Self::EntryOutOfRange { entry, blocks } => {
                write!(
                    f,
                    "entry block {entry:?} is out of range for {blocks} blocks"
                )
            }
            Self::MissingReturnLocal => f.write_str("MIR body has no return local"),
            Self::ReturnLocalHasWrongKind { kind } => {
                write!(f, "local zero has kind {kind:?}, expected the return place")
            }
            Self::ArgCountOutOfRange { arg_count, locals } => write!(
                f,
                "body declares {arg_count} arguments but has only {locals} locals"
            ),
            Self::ArgLocalHasWrongKind { local, kind } => {
                write!(f, "local {local:?} is an argument but has kind {kind:?}")
            }
            Self::UnexpectedArg { local } => {
                write!(
                    f,
                    "local {local:?} has argument kind outside the argument range"
                )
            }
            Self::PlaceOutOfRange { block, local } => {
                write!(f, "block {block:?} refers to missing local {local:?}")
            }
            Self::BlockTargetOutOfRange { block, target } => {
                write!(f, "block {block:?} jumps to missing block {target:?}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Checks one body.
pub(crate) fn validate_body(body: &Body) -> Result<(), ValidationError> {
    if body.blocks.is_empty() {
        return Err(ValidationError::NoBlocks);
    }
    if body.entry.index() as usize >= body.blocks.len() {
        return Err(ValidationError::EntryOutOfRange {
            entry: body.entry,
            blocks: body.blocks.len(),
        });
    }
    if body.locals.is_empty() {
        return Err(ValidationError::MissingReturnLocal);
    }
    if body.arg_count >= body.locals.len() {
        return Err(ValidationError::ArgCountOutOfRange {
            arg_count: body.arg_count,
            locals: body.locals.len(),
        });
    }

    for (local, declaration) in body.locals.iter_enumerated() {
        let index = local.index() as usize;
        let kind = declaration.kind;
        if index == 0 {
            if kind != LocalKind::Return {
                return Err(ValidationError::ReturnLocalHasWrongKind { kind });
            }
        } else if index <= body.arg_count {
            if kind != LocalKind::Arg {
                return Err(ValidationError::ArgLocalHasWrongKind { local, kind });
            }
        } else if kind == LocalKind::Arg {
            return Err(ValidationError::UnexpectedArg { local });
        }
    }

    for (block, data) in body.blocks.iter_enumerated() {
        for statement in &data.statements {
            let StatementKind::Assign { place, rvalue } = &statement.kind;
            check_place(body, block, place.clone())?;
            check_rvalue(body, block, rvalue)?;
        }
        match &data.terminator.kind {
            TerminatorKind::Goto(target) => check_target(body, block, *target)?,
            TerminatorKind::SwitchInt {
                discriminant,
                targets,
                otherwise,
            } => {
                check_operand(body, block, discriminant)?;
                for target in targets {
                    check_target(body, block, target.target)?;
                }
                check_target(body, block, *otherwise)?;
            }
            TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
    }

    Ok(())
}

fn check_place(body: &Body, block: BasicBlockId, place: Place) -> Result<(), ValidationError> {
    if place.local.index() as usize >= body.locals.len() {
        return Err(ValidationError::PlaceOutOfRange {
            block,
            local: place.local,
        });
    }
    Ok(())
}

fn check_operand(
    body: &Body,
    block: BasicBlockId,
    operand: &Operand,
) -> Result<(), ValidationError> {
    if let Some(place) = operand.place() {
        check_place(body, block, place)?;
    }
    Ok(())
}

fn check_rvalue(body: &Body, block: BasicBlockId, rvalue: &Rvalue) -> Result<(), ValidationError> {
    match rvalue {
        Rvalue::Use(operand) => check_operand(body, block, operand),
        Rvalue::Ref(place) => check_place(body, block, place.clone()),
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand(body, block, lhs)?;
            check_operand(body, block, rhs)
        }
        Rvalue::Unary { operand, .. } => check_operand(body, block, operand),
        Rvalue::Call { args, .. } => {
            for arg in args {
                check_operand(body, block, arg)?;
            }
            Ok(())
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                check_operand(body, block, operand)?;
            }
            Ok(())
        }
    }
}

fn check_target(
    body: &Body,
    block: BasicBlockId,
    target: BasicBlockId,
) -> Result<(), ValidationError> {
    if target.index() as usize >= body.blocks.len() {
        return Err(ValidationError::BlockTargetOutOfRange { block, target });
    }
    Ok(())
}
