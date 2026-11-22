//! The versioned public-API snapshot exchanged by the driver and registry.
//!
//! The snapshot is data, not a stable ABI. It records every public module, item,
//! signature, struct field, and enum variant so a registry can detect a breaking
//! change before publishing. Unknown fields are preserved on read and ignored on
//! write, and the digest is computed over the normalized document.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ApiError, ApiResult};

/// The only snapshot schema this crate understands.
pub const FORMAT: u32 = 1;

/// One public module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiModule {
    /// The module path, with `::` separators.
    pub path: String,
    /// Whether the module is exposed to dependent packages.
    #[serde(default)]
    pub exposed: bool,
    /// The package index the module belongs to.
    #[serde(default)]
    pub package: usize,
    /// Documentation attached to the module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// One public field.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiField {
    /// The declared field name; `None` for a positional variant field.
    #[serde(default)]
    pub name: Option<String>,
    /// The rendered field type.
    pub ty: String,
    /// The field visibility, lowercased.
    #[serde(default)]
    pub visibility: String,
    /// Documentation attached to the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// One public enum variant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiVariant {
    /// The variant name.
    pub name: String,
    /// Fields in canonical (name) order.
    #[serde(default)]
    pub fields: Vec<ApiField>,
    /// Documentation attached to the variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// Size and alignment of an aggregate, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ApiLayout {
    /// Size in bytes, including padding.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u64,
}

/// One implementation attached to an aggregate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiImplementation {
    /// The implemented trait, or `None` for an inherent implementation.
    #[serde(default, rename = "trait", skip_serializing_if = "Option::is_none")]
    pub trait_name: Option<String>,
    /// Method names in declaration order.
    #[serde(default)]
    pub methods: Vec<String>,
}

/// One public item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiItem {
    /// The item name.
    pub name: String,
    /// The item kind, such as `function`, `struct`, or `enum`.
    pub kind: String,
    /// The declaring module path.
    pub module: String,
    /// The canonical `module::name` target of a re-exported item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical: Option<String>,
    /// The item visibility, lowercased.
    #[serde(default)]
    pub visibility: String,
    /// The source `cfg` condition, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<String>,
    /// The rendered signature for functions, methods, and globals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Public struct fields in canonical (name) order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ApiField>,
    /// Enum variants in canonical (name) order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<ApiVariant>,
    /// Documentation attached to the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// Aggregate layout, when the item is a sized struct or enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<ApiLayout>,
    /// Inherent and trait implementations attached to the item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<ApiImplementation>,
}

/// The complete public-API snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiSnapshot {
    /// The snapshot schema version.
    pub version: u32,
    /// The package name.
    pub package: String,
    /// The package version.
    #[serde(default)]
    pub package_version: String,
    /// The target triple the snapshot was produced for.
    pub target: String,
    /// Public modules, sorted by path.
    #[serde(default)]
    pub modules: Vec<ApiModule>,
    /// Public items, sorted by module and name.
    #[serde(default)]
    pub items: Vec<ApiItem>,
}

impl ApiSnapshot {
    /// Parses and validates a snapshot document.
    pub fn parse(bytes: &[u8]) -> ApiResult<Self> {
        let snapshot: Self = serde_json::from_slice(bytes)
            .map_err(|error| ApiError::snapshot(format!("invalid API snapshot: {error}")))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Reads a snapshot from disk.
    pub fn read(path: &std::path::Path) -> ApiResult<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            ApiError::snapshot(format!("cannot read `{}`: {error}", path.display()))
        })?;
        Self::parse(&bytes)
    }

    /// Validates schema, identity, and key uniqueness.
    pub fn validate(&self) -> ApiResult<()> {
        if self.version != FORMAT {
            return Err(ApiError::snapshot(format!(
                "the API snapshot uses version {}, but this tool understands {FORMAT}",
                self.version
            )));
        }
        if self.package.is_empty() || self.package.chars().any(char::is_control) {
            return Err(ApiError::snapshot(
                "the API snapshot has an empty or invalid package name",
            ));
        }
        if self.target.is_empty() {
            return Err(ApiError::snapshot(
                "the API snapshot has an empty target triple",
            ));
        }
        let mut modules = std::collections::BTreeSet::new();
        for module in &self.modules {
            if module.path.is_empty() || !modules.insert(module.path.as_str()) {
                return Err(ApiError::snapshot(format!(
                    "the API snapshot declares the module `{}` twice or with an empty path",
                    module.path
                )));
            }
        }
        let mut keys = std::collections::BTreeSet::new();
        for item in &self.items {
            if item.name.is_empty() || item.module.is_empty() || item.kind.is_empty() {
                return Err(ApiError::snapshot(
                    "an API item has an empty name, kind, or module",
                ));
            }
            if !keys.insert(item.key()) {
                return Err(ApiError::snapshot(format!(
                    "the API snapshot declares `{}` twice",
                    item.key()
                )));
            }
            let mut fields = std::collections::BTreeSet::new();
            for field in &item.fields {
                let name = field.name.clone().unwrap_or_else(|| "#".to_owned());
                if !fields.insert(name.clone()) {
                    return Err(ApiError::snapshot(format!(
                        "`{}` declares the field `{name}` twice",
                        item.key()
                    )));
                }
            }
            let mut variants = std::collections::BTreeSet::new();
            for variant in &item.variants {
                if variant.name.is_empty() || !variants.insert(variant.name.as_str()) {
                    return Err(ApiError::snapshot(format!(
                        "`{}` declares an empty or duplicate variant",
                        item.key()
                    )));
                }
                let mut names = std::collections::BTreeSet::new();
                for field in &variant.fields {
                    let name = field.name.clone().unwrap_or_else(|| "#".to_owned());
                    if !names.insert(name.clone()) {
                        return Err(ApiError::snapshot(format!(
                            "the variant `{}::{}` declares the field `{name}` twice",
                            item.key(),
                            variant.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns the items keyed by module, name, and kind.
    pub fn item_map(&self) -> BTreeMap<String, &ApiItem> {
        self.items.iter().map(|item| (item.key(), item)).collect()
    }

    /// Returns the modules keyed by path.
    pub fn module_map(&self) -> BTreeMap<&str, &ApiModule> {
        self.modules
            .iter()
            .map(|module| (module.path.as_str(), module))
            .collect()
    }

    /// Returns the canonical JSON bytes used for hashing.
    pub fn canonical_bytes(&self) -> ApiResult<Vec<u8>> {
        let value = serde_json::to_value(self).map_err(|error| {
            ApiError::snapshot(format!("cannot encode the API snapshot: {error}"))
        })?;
        Ok(canonical_json(&value))
    }

    /// Returns the `sha256:<hex>` digest of the canonical document.
    pub fn hash(&self) -> ApiResult<String> {
        let bytes = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
    }

    /// Sorts modules and items deterministically.
    pub fn normalize(&mut self) {
        self.modules
            .sort_by(|left, right| left.path.cmp(&right.path));
        for item in &mut self.items {
            item.fields.sort();
            item.variants.sort();
            item.implementations.sort_by(|left, right| {
                left.trait_name
                    .cmp(&right.trait_name)
                    .then_with(|| left.methods.cmp(&right.methods))
            });
            for variant in &mut item.variants {
                variant.fields.sort();
            }
        }
        self.items.sort_by(|left, right| {
            left.module
                .cmp(&right.module)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.kind.cmp(&right.kind))
        });
    }
}

impl ApiItem {
    /// Returns the stable identity of the item.
    pub fn key(&self) -> String {
        format!("{}::{}#{}", self.module, self.name, self.kind)
    }
}

fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    let mut output = String::new();
    write_canonical(value, &mut output);
    output.into_bytes()
}

fn write_canonical(value: &serde_json::Value, output: &mut String) {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(number) => output.push_str(&number.to_string()),
        serde_json::Value::String(value) => output
            .push_str(&serde_json::to_string(value).expect("a JSON string is always serializable")),
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical(value, output);
            }
            output.push(']');
        }
        serde_json::Value::Object(values) => {
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("a JSON key is always serializable"),
                );
                output.push(':');
                write_canonical(value, output);
            }
            output.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> ApiSnapshot {
        ApiSnapshot {
            version: 1,
            package: "demo".to_owned(),
            package_version: "1.0.0".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: vec![ApiModule {
                path: "root".to_owned(),
                exposed: true,
                package: 0,
                docs: None,
            }],
            items: vec![ApiItem {
                name: "Point".to_owned(),
                kind: "struct".to_owned(),
                module: "root".to_owned(),
                canonical: None,
                visibility: "public".to_owned(),
                cfg: None,
                signature: None,
                fields: vec![ApiField {
                    name: Some("x".to_owned()),
                    ty: "i32".to_owned(),
                    visibility: "public".to_owned(),
                    docs: None,
                }],
                variants: Vec::new(),
                docs: None,
                layout: None,
                implementations: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_snapshot_round_trips_and_hashes_deterministically() {
        let snapshot = snapshot();
        let bytes = snapshot.canonical_bytes().unwrap();
        let parsed = ApiSnapshot::parse(&bytes).unwrap();
        assert_eq!(parsed, snapshot);
        assert_eq!(snapshot.hash().unwrap(), parsed.hash().unwrap());
        assert!(snapshot.hash().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn an_unknown_schema_is_rejected() {
        let text = br#"{"version": 9, "package": "demo", "target": "x"}"#;
        let error = ApiSnapshot::parse(text).unwrap_err();
        assert_eq!(error.code(), "E8090");
    }

    #[test]
    fn duplicate_items_are_rejected() {
        let mut snapshot = snapshot();
        snapshot.items.push(snapshot.items[0].clone());
        assert!(snapshot.validate().is_err());
    }

    #[test]
    fn hashing_ignores_field_order() {
        let first = snapshot();
        let mut second = snapshot();
        second.items[0].fields.push(ApiField {
            name: Some("y".to_owned()),
            ty: "i64".to_owned(),
            visibility: "public".to_owned(),
            docs: None,
        });
        second.items[0].fields.sort();
        let mut third = snapshot();
        third.items[0].fields.insert(
            0,
            ApiField {
                name: Some("y".to_owned()),
                ty: "i64".to_owned(),
                visibility: "public".to_owned(),
                docs: None,
            },
        );
        third.normalize();
        assert_eq!(second.hash().unwrap(), third.hash().unwrap());
        assert_ne!(first.hash().unwrap(), second.hash().unwrap());
    }
}
