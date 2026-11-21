//! Runtime and panic policies declared by a target.

use std::fmt;

/// Whether a target links the Seal runtime.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum RuntimePolicy {
    /// Link the runtime archive that provides allocation, panics, and exports.
    #[default]
    Default,
    /// Do not link any runtime. The program is freestanding.
    None,
}

impl RuntimePolicy {
    /// Returns the spelling used in manifests, plans, and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::None => "none",
        }
    }

    /// Parses a manifest spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

impl fmt::Display for RuntimePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a panic does on the selected target.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PanicStrategy {
    /// Call the runtime's `seal_panic`, which reports the location and aborts.
    #[default]
    Abort,
    /// Emit an LLVM `unreachable`, which traps without a runtime.
    Trap,
}

impl PanicStrategy {
    /// Returns the spelling used in manifests, plans, and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Abort => "abort",
            Self::Trap => "trap",
        }
    }

    /// Parses a manifest spelling.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "abort" => Some(Self::Abort),
            "trap" => Some(Self::Trap),
            _ => None,
        }
    }
}

impl fmt::Display for PanicStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{PanicStrategy, RuntimePolicy};

    #[test]
    fn policies_round_trip_through_their_spelling() {
        for runtime in [RuntimePolicy::Default, RuntimePolicy::None] {
            assert_eq!(RuntimePolicy::parse(runtime.as_str()), Some(runtime));
        }
        for panic in [PanicStrategy::Abort, PanicStrategy::Trap] {
            assert_eq!(PanicStrategy::parse(panic.as_str()), Some(panic));
        }
    }

    #[test]
    fn unknown_policies_are_rejected() {
        assert_eq!(RuntimePolicy::parse("alloc"), None);
        assert_eq!(PanicStrategy::parse("unwind"), None);
    }
}
