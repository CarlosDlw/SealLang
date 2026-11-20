//! The kinds of definition.

/// What a definition introduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefKind {
    /// A function item.
    Function,
    /// A constant item or block-scope constant declaration.
    Const,
    /// A static item or block-scope static declaration.
    Static,
    /// A function declared in an `extern` declaration.
    ExternFunction,
    /// A static declared in an `extern` declaration.
    ExternStatic,
    /// A struct item.
    Struct,
    /// An opaque type declared by `extern type`.
    OpaqueType,
    /// An enum item.
    Enum,
    /// An enum variant declaration.
    Variant,
    /// A trait item.
    Trait,
    /// A method signature declared by a trait.
    TraitMethod,
    /// An implementation block.
    Impl,
    /// A generic type parameter declaration.
    GenericParam,
    /// A struct field declaration.
    Field,
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
            Self::Const => "constant",
            Self::Static => "static",
            Self::ExternFunction => "external function",
            Self::ExternStatic => "external static",
            Self::Struct => "struct",
            Self::OpaqueType => "opaque type",
            Self::Enum => "enum",
            Self::Variant => "variant",
            Self::Trait => "trait",
            Self::TraitMethod => "trait method",
            Self::Impl => "implementation",
            Self::GenericParam => "type parameter",
            Self::Field => "field",
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
        assert!(!DefKind::OpaqueType.is_local());
        assert!(!DefKind::Enum.is_local());
        assert!(!DefKind::Variant.is_local());
        assert!(!DefKind::Trait.is_local());
        assert!(!DefKind::Const.is_local());
        assert!(!DefKind::Static.is_local());
        assert!(!DefKind::ExternFunction.is_local());
        assert!(!DefKind::ExternStatic.is_local());
        assert!(DefKind::Param.is_local());
        assert!(DefKind::Binding.is_local());
    }

    #[test]
    fn descriptions_are_distinct() {
        let descriptions = [
            DefKind::Function.description(),
            DefKind::Const.description(),
            DefKind::Static.description(),
            DefKind::ExternFunction.description(),
            DefKind::ExternStatic.description(),
            DefKind::Struct.description(),
            DefKind::OpaqueType.description(),
            DefKind::Enum.description(),
            DefKind::Variant.description(),
            DefKind::Trait.description(),
            DefKind::TraitMethod.description(),
            DefKind::Impl.description(),
            DefKind::GenericParam.description(),
            DefKind::Field.description(),
            DefKind::Param.description(),
            DefKind::Binding.description(),
        ];
        let mut sorted = descriptions.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), descriptions.len());
    }
}
