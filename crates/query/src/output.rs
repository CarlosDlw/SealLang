//! The value and fingerprint returned by a query.

use std::fmt;
use std::sync::Arc;

use seal_hash::Fingerprint;

/// A computed value together with its semantic fingerprint.
///
/// Two equal semantic results must have the same fingerprint. Callers normally
/// use [`seal_hash::stable_hash_of`] or hash the serialized form of the value.
/// The fingerprint is the only signal used for backdating; it must not include
/// timestamps, allocations, or host state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryOutput<T> {
    value: T,
    fingerprint: Fingerprint,
}

impl<T> QueryOutput<T> {
    /// Creates an output from a value and its semantic fingerprint.
    #[inline]
    pub const fn new(value: T, fingerprint: Fingerprint) -> Self {
        Self { value, fingerprint }
    }

    /// Returns the value by reference.
    #[inline]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns the semantic fingerprint.
    #[inline]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Splits the output into its value and fingerprint.
    #[inline]
    pub fn into_parts(self) -> (T, Fingerprint) {
        (self.value, self.fingerprint)
    }
}

/// The result of one database query.
///
/// `executed` is false for a cache hit. `backdated` is true when the query ran
/// but produced the same fingerprint as before, so its dependents stayed valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryOutcome<T> {
    value: Arc<T>,
    fingerprint: Fingerprint,
    executed: bool,
    backdated: bool,
}

impl<T> QueryOutcome<T> {
    pub(crate) const fn new(
        value: Arc<T>,
        fingerprint: Fingerprint,
        executed: bool,
        backdated: bool,
    ) -> Self {
        Self {
            value,
            fingerprint,
            executed,
            backdated,
        }
    }

    /// Returns the shared value.
    #[inline]
    pub fn value(&self) -> &Arc<T> {
        &self.value
    }

    /// Returns the semantic fingerprint.
    #[inline]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    /// Returns whether the query was computed during this request.
    #[inline]
    pub const fn executed(&self) -> bool {
        self.executed
    }

    /// Returns whether a recomputation produced the previous fingerprint.
    #[inline]
    pub const fn backdated(&self) -> bool {
        self.backdated
    }

    /// Splits the outcome into its shared value and fingerprint.
    #[inline]
    pub fn into_parts(self) -> (Arc<T>, Fingerprint) {
        (self.value, self.fingerprint)
    }
}

impl<T> fmt::Display for QueryOutcome<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.fingerprint)
    }
}
