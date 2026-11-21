//! Typed query failures.

use std::fmt;

use crate::key::{InputId, QueryKey};

/// A failure to read or execute a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// An input identifier does not belong to this database.
    UnknownInput(InputId),
    /// An input name is empty; logical identities must be non-empty.
    EmptyInputName,
    /// The database exhausted its 32-bit input identifier space.
    TooManyInputs,
    /// A dependency query has never been computed.
    MissingDependency(QueryKey),
    /// The same key was used with a different value type.
    TypeMismatch {
        /// The offending key.
        key: QueryKey,
    },
    /// A query returned the reserved zero fingerprint.
    InvalidFingerprint {
        /// The offending key.
        key: QueryKey,
    },
    /// A query recursively depended on itself.
    Cycle {
        /// The keys on the active path.
        path: Vec<QueryKey>,
    },
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownInput(id) => write!(f, "unknown query input {id}"),
            Self::EmptyInputName => f.write_str("a query input name cannot be empty"),
            Self::TooManyInputs => f.write_str("the query database has too many inputs"),
            Self::MissingDependency(key) => {
                write!(f, "the dependency `{key}` has not been computed")
            }
            Self::TypeMismatch { key } => {
                write!(
                    f,
                    "the query `{key}` was reused with a different value type"
                )
            }
            Self::InvalidFingerprint { key } => {
                write!(
                    f,
                    "the query `{key}` returned the reserved zero fingerprint"
                )
            }
            Self::Cycle { path } => {
                f.write_str("query cycle: ")?;
                for (index, key) in path.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" -> ")?;
                    }
                    write!(f, "{key}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for QueryError {}
