//! Package feature declarations and their conflict table.
//!
//! A feature is a named, additive request. Its values may enable another
//! feature (`std`), activate an optional dependency (`dep:serde`), or request a
//! feature from that dependency (`serde/derive`, `serde?/derive`). The
//! `conflicts` table is not a feature: it declares pairs that may not be
//! enabled together.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The `[package.features]` table.
#[derive(Clone, Debug, Default)]
pub struct FeatureManifest {
    entries: BTreeMap<String, FeatureEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum FeatureEntry {
    /// A normal feature and the requests it enables.
    Values(Vec<String>),
    /// The nested conflict table.
    Conflicts(BTreeMap<String, Vec<String>>),
}

impl<'de> Deserialize<'de> for FeatureManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let entries = BTreeMap::<String, FeatureEntry>::deserialize(deserializer)?;
        Ok(Self { entries })
    }
}

impl FeatureManifest {
    /// Returns an empty feature table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns every declared feature and its values, excluding `conflicts`.
    pub fn values(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.entries.iter().filter_map(|(name, entry)| match entry {
            FeatureEntry::Values(values) => Some((name.as_str(), values.as_slice())),
            FeatureEntry::Conflicts(_) => None,
        })
    }

    /// Returns the declared feature names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values().map(|(name, _)| name)
    }

    /// Returns the values of one feature.
    pub fn get(&self, name: &str) -> Option<&[String]> {
        self.entries.get(name).and_then(|entry| match entry {
            FeatureEntry::Values(values) => Some(values.as_slice()),
            FeatureEntry::Conflicts(_) => None,
        })
    }

    /// Returns whether a feature is declared.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns every declared conflict, keyed by the feature that declares it.
    pub fn conflict_map(&self) -> BTreeMap<&str, &[String]> {
        let mut result = BTreeMap::new();
        for entry in self.entries.values() {
            if let FeatureEntry::Conflicts(conflicts) = entry {
                for (name, values) in conflicts {
                    result.insert(name.as_str(), values.as_slice());
                }
            }
        }
        result
    }
}
