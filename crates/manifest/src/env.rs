//! Declared environment values.

use serde::Deserialize;

/// An environment value written as a string or as a table.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum EnvValue {
    /// A literal value.
    Literal(String),
    /// A value with tracking and secrecy metadata.
    Detailed(EnvDetail),
}

impl EnvValue {
    /// Returns the literal value.
    pub fn value(&self) -> &str {
        match self {
            Self::Literal(value) => value,
            Self::Detailed(detail) => &detail.value,
        }
    }

    /// Returns whether the value participates in the build key.
    pub fn tracked(&self) -> bool {
        match self {
            Self::Literal(_) => false,
            Self::Detailed(detail) => detail.tracked,
        }
    }

    /// Returns whether the value must never be logged.
    pub fn secret(&self) -> bool {
        match self {
            Self::Literal(_) => false,
            Self::Detailed(detail) => detail.secret,
        }
    }
}

/// The table form of an environment value.
#[derive(Clone, Debug, Deserialize)]
pub struct EnvDetail {
    /// The literal value.
    pub value: String,
    /// Whether a change invalidates cached actions.
    #[serde(default)]
    pub tracked: bool,
    /// Whether the value is secret.
    #[serde(default)]
    pub secret: bool,
}
