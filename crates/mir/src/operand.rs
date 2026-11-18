//! Constants and operands.

use seal_def::DefId;
use seal_symbol::Symbol;
use seal_ty::TyId;

use crate::place::Place;

/// A literal value embedded in a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constant {
    /// An integer literal with the type selected by the checker.
    Int {
        /// The value, widened to `u128`.
        value: u128,
        /// The signed or unsigned integer type.
        ty: TyId,
    },
    /// A floating-point literal with the type selected by the checker.
    Float {
        /// The value.
        value: f64,
        /// `f32` or `f64`.
        ty: TyId,
    },
    /// A boolean literal.
    Bool(bool),
    /// A character literal.
    Char(char),
    /// A string literal; the symbol holds the unescaped contents.
    Str {
        /// The unescaped contents.
        value: Symbol,
        /// The `*str` type selected by the checker.
        ty: TyId,
    },
    /// The unit value.
    Unit,
}

/// A value read by an operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// Reads a place without taking ownership; valid because the type is `Copy`.
    Copy(Place),
    /// Reads a place and moves ownership out of it.
    Move(Place),
    /// Uses a literal directly.
    Constant(Constant),
    /// Reads a constant item's value.
    ///
    /// The value is produced by constant evaluation, so this operand has no
    /// place and cannot be written or borrowed.
    Const(DefId),
}

impl Operand {
    /// Returns the place this operand reads, if it reads one.
    #[inline]
    pub fn place(&self) -> Option<Place> {
        match self {
            Self::Copy(place) | Self::Move(place) => Some(place.clone()),
            Self::Constant(_) | Self::Const(_) => None,
        }
    }
}
