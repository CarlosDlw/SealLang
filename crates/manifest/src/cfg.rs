//! Typed conditions for dependency and target activation.
//!
//! Manifest conditions are tables, never strings: `{ os = "linux" }`,
//! `{ any = [{ os = "linux" }, { os = "windows" }] }`, and so on. A typo is a
//! parse error instead of a condition that silently evaluates to false.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// A condition attached to a dependency or target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgSpec {
    /// `os = "..."`.
    Os(String),
    /// `arch = "..."`.
    Arch(String),
    /// `env = "..."` (the target environment, not a process variable).
    Env(String),
    /// `pointer-width = 64`.
    PointerWidth(u8),
    /// Every child must match.
    All(Vec<CfgSpec>),
    /// At least one child must match.
    Any(Vec<CfgSpec>),
    /// The child must not match.
    Not(Box<CfgSpec>),
}

impl<'de> Deserialize<'de> for CfgSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = toml::Value::deserialize(deserializer)?;
        from_value(value).map_err(D::Error::custom)
    }
}

fn from_value(value: toml::Value) -> Result<CfgSpec, String> {
    let toml::Value::Table(table) = value else {
        return Err("a `when` condition must be a table".to_owned());
    };
    let mut conditions = Vec::new();
    for (key, value) in table {
        match key.as_str() {
            "any" => conditions.push(CfgSpec::Any(children(value, "any")?)),
            "all" => conditions.push(CfgSpec::All(children(value, "all")?)),
            "not" => conditions.push(CfgSpec::Not(Box::new(from_value(value)?))),
            "os" => conditions.push(CfgSpec::Os(string(value, "os")?)),
            "arch" => conditions.push(CfgSpec::Arch(string(value, "arch")?)),
            "env" => conditions.push(CfgSpec::Env(string(value, "env")?)),
            "pointer-width" => {
                let Some(width) = value.as_integer() else {
                    return Err("`pointer-width` must be an integer".to_owned());
                };
                let width = u8::try_from(width)
                    .map_err(|_| "`pointer-width` must be between 0 and 255".to_owned())?;
                conditions.push(CfgSpec::PointerWidth(width));
            }
            _ => {
                return Err(format!(
                    "unknown `when` key `{key}`; use `os`, `arch`, `env`, `pointer-width`, `any`, `all`, or `not`"
                ));
            }
        }
    }
    match conditions.len() {
        0 => Err("a `when` condition cannot be empty".to_owned()),
        1 => Ok(conditions.remove(0)),
        _ => Ok(CfgSpec::All(conditions)),
    }
}

fn children(value: toml::Value, key: &str) -> Result<Vec<CfgSpec>, String> {
    let Some(array) = value.as_array() else {
        return Err(format!("`{key}` must be an array of conditions"));
    };
    array
        .iter()
        .cloned()
        .map(from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("in `{key}`: {error}"))
}

fn string(value: toml::Value, key: &str) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("`{key}` must be a string"))
}
