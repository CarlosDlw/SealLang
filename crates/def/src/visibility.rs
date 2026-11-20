//! Visibility boundaries shared by every semantic phase.

/// The declared visibility of an item or field.
///
/// Visibility is a property of the declaration, not of a use: name resolution
/// decides whether a declaration is reachable from another module or package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Visible in the declaring module and its descendants.
    #[default]
    Private,
    /// Visible anywhere in the declaring package.
    Internal,
    /// Visible outside the package when the declaring module is exposed.
    Public,
}

impl Visibility {
    /// Returns the source spelling of the visibility.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Internal => "internal",
            Self::Public => "public",
        }
    }

    /// Returns `true` when the visibility reaches outside the package.
    #[inline]
    pub const fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }

    /// Returns `true` when a declaration is reachable from anywhere in its
    /// package.
    #[inline]
    pub const fn is_package_wide(self) -> bool {
        matches!(self, Self::Internal | Self::Public)
    }
}
