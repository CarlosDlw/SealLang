//! Calling conventions named by an `extern` declaration.
//!
//! The ABI spelling is part of the declaration, not of the target: a program
//! asks for a convention and the compiler either supports it or rejects the
//! declaration. The enum is closed so a new convention becomes a variant with
//! its own availability rule instead of a new string comparison scattered
//! through the compiler.

use std::fmt;

/// A calling convention written after `extern`.
///
/// Every variant currently maps to the target's default C convention. A
/// convention is only constructible through [`Abi::from_name`], which is the
/// single place that decides which spellings exist.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Abi {
    /// `"c"`: the target's default C convention.
    C,
    /// `"cdecl"`: the C convention on every supported target.
    Cdecl,
    /// `"system"`: the platform convention, which is the C convention here.
    System,
}

impl Abi {
    /// Parses an ABI spelling.
    ///
    /// Returns `None` for an unknown or unavailable convention. `stdcall` and
    /// `fastcall` are not accepted yet: they have no target-independent meaning
    /// and the compiler cannot emit them.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "c" => Self::C,
            "cdecl" => Self::Cdecl,
            "system" => Self::System,
            _ => return None,
        })
    }

    /// Returns the spelling used in diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cdecl => "cdecl",
            Self::System => "system",
        }
    }

    /// Returns the code generation convention for this ABI.
    pub const fn call_conv(self) -> CallConv {
        CallConv::C
    }
}

impl fmt::Display for Abi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The low-level calling convention a function is declared with.
///
/// Code generation maps this to the backend's convention instead of deriving it
/// from `Abi` at each use.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CallConv {
    /// The target's default C convention.
    C,
}

impl CallConv {
    /// Returns the spelling used in debug output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C => "c",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Abi, CallConv};

    #[test]
    fn the_c_family_is_recognized() {
        for name in ["c", "cdecl", "system"] {
            let abi = Abi::from_name(name).expect("the ABI is recognized");
            assert_eq!(abi.as_str(), name);
            assert_eq!(abi.call_conv(), CallConv::C);
        }
    }

    #[test]
    fn unknown_and_unavailable_abis_are_rejected() {
        for name in ["", "C", "c-unwind", "stdcall", "fastcall", "thiscall"] {
            assert!(Abi::from_name(name).is_none(), "`{name}` must be rejected");
        }
    }
}
