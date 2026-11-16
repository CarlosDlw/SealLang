//! The kinds of definition.

/// What a definition introduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefKind {
    /// A function item.
    Function,
    /// A function parameter.
    Param,
    /// A `let` binding.
    Binding,
}

impl DefKind {
    /// Returns `true` for definitions that live inside a function body.
    #[inline]
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Param | Self::Binding)
    }

    /// Returns a human-readable description for diagnostics.
    #[inline]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Param => "parameter",
            Self::Binding => "binding",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DefKind;

    #[test]
    fn only_parameters_and_bindings_are_local() {
        assert!(!DefKind::Function.is_local());
        assert!(DefKind::Param.is_local());
        assert!(DefKind::Binding.is_local());
    }

    #[test]
    fn descriptions_are_distinct() {
        let descriptions = [
            DefKind::Function.description(),
            DefKind::Param.description(),
            DefKind::Binding.description(),
        ];
        let mut sorted = descriptions.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), descriptions.len());
    }
}
