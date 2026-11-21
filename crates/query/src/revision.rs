//! Input revisions and recorded dependencies.

use std::fmt;

use seal_hash::Fingerprint;

use crate::key::{InputId, InputKind, QueryKey};

/// A monotonic revision number.
///
/// Revision zero is the initial state. Every input whose bytes change advances
/// the database revision once; setting identical bytes does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    /// The initial revision.
    pub const INITIAL: Self = Self(0);

    /// Creates a revision from its raw value.
    #[inline]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    #[inline]
    pub const fn as_raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A snapshot of one input's current identity and contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStamp {
    id: InputId,
    kind: InputKind,
    name: String,
    revision: Revision,
    digest: Fingerprint,
}

impl InputStamp {
    pub(crate) fn new(
        id: InputId,
        kind: InputKind,
        name: String,
        revision: Revision,
        digest: Fingerprint,
    ) -> Self {
        Self {
            id,
            kind,
            name,
            revision,
            digest,
        }
    }

    /// Returns the input identifier.
    #[inline]
    pub const fn id(&self) -> InputId {
        self.id
    }

    /// Returns the input kind.
    #[inline]
    pub const fn kind(&self) -> InputKind {
        self.kind
    }

    /// Returns the logical name.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the revision at which the current contents were installed.
    #[inline]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the content fingerprint.
    #[inline]
    pub const fn digest(&self) -> Fingerprint {
        self.digest
    }
}

impl fmt::Display for InputStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}@{}", self.kind, self.name, self.revision)
    }
}

/// A derived query's current result identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryStamp {
    key: QueryKey,
    fingerprint: Fingerprint,
}

impl QueryStamp {
    pub(crate) fn new(key: QueryKey, fingerprint: Fingerprint) -> Self {
        Self { key, fingerprint }
    }

    /// Returns the query key.
    #[inline]
    pub const fn key(&self) -> &QueryKey {
        &self.key
    }

    /// Returns the result fingerprint.
    #[inline]
    pub const fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

impl fmt::Display for QueryStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.key, self.fingerprint)
    }
}

/// One dependency read by a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    /// An input at a particular content fingerprint.
    Input(InputStamp),
    /// A derived query at a particular result fingerprint.
    Query(QueryStamp),
}

impl Dependency {
    /// Returns the fingerprint recorded when the dependency was read.
    #[inline]
    pub const fn fingerprint(&self) -> Fingerprint {
        match self {
            Self::Input(input) => input.digest,
            Self::Query(query) => query.fingerprint,
        }
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(input) => input.fmt(f),
            Self::Query(query) => query.fmt(f),
        }
    }
}
