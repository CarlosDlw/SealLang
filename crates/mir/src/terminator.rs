//! Terminators, which end basic blocks and carry control-flow edges.

use seal_span::Span;

use crate::id::BasicBlockId;
use crate::operand::Operand;

/// A value-to-block edge of a `SwitchInt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchTarget {
    /// The discriminant value.
    pub value: u128,
    /// The block entered when the discriminant equals `value`.
    pub target: BasicBlockId,
}

/// What happens at the end of a basic block.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminatorKind {
    /// Jumps to one block.
    Goto(BasicBlockId),
    /// Chooses a block from the discriminant.
    ///
    /// Boolean conditions use the values `1` (true) and `0` (false).
    SwitchInt {
        /// The value being tested.
        discriminant: Operand,
        /// Case values in source order.
        targets: Vec<SwitchTarget>,
        /// The block used when no case matches.
        otherwise: BasicBlockId,
    },
    /// Returns from the function, using the return place as the value.
    Return,
    /// Control never reaches this point.
    Unreachable,
}

impl TerminatorKind {
    /// Returns the blocks this terminator can enter, in source order.
    pub fn successors(&self) -> Vec<BasicBlockId> {
        match self {
            Self::Goto(target) => vec![*target],
            Self::SwitchInt {
                targets, otherwise, ..
            } => targets
                .iter()
                .map(|target| target.target)
                .chain(std::iter::once(*otherwise))
                .collect(),
            Self::Return | Self::Unreachable => Vec::new(),
        }
    }
}

/// A terminator together with its source location.
#[derive(Clone, Debug, PartialEq)]
pub struct Terminator {
    /// What happens.
    pub kind: TerminatorKind,
    /// The source range this terminator came from.
    pub span: Span,
}
