//! Language version and feature-gate policy.
//!
//! Parsing and type checking consult the session's [`LanguageVersion`] and
//! [`FeatureGates`] whenever they accept syntax or behavior that is not stable
//! yet. Keeping both in one crate means the policy is identical in every phase
//! and can be tested without a compiler pipeline.

#![warn(missing_docs)]

use std::collections::BTreeSet;
use std::fmt;

/// A semantic version of the language itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LanguageVersion {
    major: u32,
    minor: u32,
}

impl LanguageVersion {
    /// Version `0.1`, the current language version.
    pub const CURRENT: Self = Self { major: 0, minor: 1 };

    /// Creates a version.
    #[inline]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns the major component.
    #[inline]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the minor component.
    #[inline]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Parses `major.minor`.
    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let Some((major, minor)) = text.split_once('.') else {
            return Err(VersionError::Malformed {
                text: text.to_owned(),
            });
        };
        let parse_component = |component: &str| {
            if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            component.parse::<u32>().ok()
        };
        match (parse_component(major), parse_component(minor)) {
            (Some(major), Some(minor)) => Ok(Self { major, minor }),
            _ => Err(VersionError::Malformed {
                text: text.to_owned(),
            }),
        }
    }
}

impl fmt::Display for LanguageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// A year-named language edition.
///
/// An edition is a compatibility line, not a compiler version. Every package
/// selects its own edition, so a dependency keeps the semantics it was written
/// against while its dependents move to a newer line.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Edition(u16);

impl Edition {
    /// Edition `2026`, the current compatibility line.
    pub const CURRENT: Self = Self(2026);

    /// Every edition this compiler supports, in numeric order.
    pub const ALL: &'static [Self] = &[Self::CURRENT];

    /// Creates an edition from a year. The value may be unsupported; use
    /// [`Self::is_supported`] before compiling with it.
    #[inline]
    pub const fn new(year: u16) -> Self {
        Self(year)
    }

    /// Returns the four-digit year.
    #[inline]
    pub const fn year(self) -> u16 {
        self.0
    }

    /// Returns `true` when this compiler implements the edition.
    #[inline]
    pub const fn is_supported(self) -> bool {
        matches!(self.0, 2026)
    }

    /// Parses a four-digit edition name.
    pub fn parse(text: &str) -> Result<Self, EditionError> {
        if text.len() != 4 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(EditionError::Malformed {
                text: text.to_owned(),
            });
        }
        let year = text
            .bytes()
            .fold(0u16, |value, byte| value * 10 + u16::from(byte - b'0'));
        let edition = Self(year);
        if edition.is_supported() {
            Ok(edition)
        } else {
            Err(EditionError::Unsupported { year })
        }
    }

    /// Returns whether `edition` governs `feature`.
    ///
    /// The table is closed and currently empty: edition `2026` introduces no
    /// breaking change. An accepted RFC adds a variant when it ratifies one.
    #[inline]
    pub const fn supports(self, feature: EditionFeature) -> bool {
        let _ = self;
        match feature {}
    }
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A breaking change gated by edition.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum EditionFeature {}

/// Failure to parse an edition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditionError {
    /// The text was not four decimal digits.
    Malformed {
        /// The rejected text.
        text: String,
    },
    /// The year is well formed but this compiler does not implement it.
    Unsupported {
        /// The rejected year.
        year: u16,
    },
}

impl fmt::Display for EditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { text } => {
                write!(f, "invalid edition `{text}`: expected a four-digit year")
            }
            Self::Unsupported { year } => {
                write!(
                    f,
                    "the edition `{year}` is not supported; supported editions: "
                )?;
                for (index, edition) in Edition::ALL.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{edition}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for EditionError {}

/// Failure to parse a language version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionError {
    /// The text was not `major.minor`.
    Malformed {
        /// The rejected text.
        text: String,
    },
}

impl VersionError {
    /// Returns the rejected text.
    pub fn text(&self) -> &str {
        match self {
            Self::Malformed { text } => text,
        }
    }
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid language version `{}`: expected `major.minor`",
            self.text()
        )
    }
}

impl std::error::Error for VersionError {}

/// Whether a feature is stable in the selected language version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FeatureStability {
    /// Accepted without any flag.
    Stable,
    /// Requires `--allow-unstable-features`.
    Unstable,
}

/// The set of enabled features and whether unstable ones are allowed.
#[derive(Clone, Debug, Default)]
pub struct FeatureGates {
    allow_unstable: bool,
    enabled: BTreeSet<&'static str>,
}

/// A feature gate rejected the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateError {
    /// The feature is unstable and the flag was not passed.
    UnstableDisabled {
        /// Feature name.
        feature: &'static str,
    },
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnstableDisabled { feature } => write!(
                f,
                "the feature `{feature}` is unstable and requires \
                 `--allow-unstable-features`"
            ),
        }
    }
}

impl std::error::Error for GateError {}

impl FeatureGates {
    /// Creates an empty gate set.
    pub fn new(allow_unstable: bool) -> Self {
        Self {
            allow_unstable,
            enabled: BTreeSet::new(),
        }
    }

    /// Returns `true` if unstable features are allowed.
    #[inline]
    pub fn allows_unstable(&self) -> bool {
        self.allow_unstable
    }

    /// Enables a feature explicitly.
    pub fn enable(&mut self, feature: &'static str) {
        self.enabled.insert(feature);
    }

    /// Returns `true` if the feature is enabled explicitly.
    #[inline]
    pub fn is_enabled(&self, feature: &str) -> bool {
        self.enabled.contains(feature)
    }

    /// Checks that `feature` may be used, given its stability.
    ///
    /// Stable features always pass; unstable features pass only when unstable
    /// features are allowed or the feature was enabled explicitly.
    pub fn require(
        &self,
        feature: &'static str,
        stability: FeatureStability,
    ) -> Result<(), GateError> {
        match stability {
            FeatureStability::Stable => Ok(()),
            FeatureStability::Unstable if self.allow_unstable || self.enabled.contains(feature) => {
                Ok(())
            }
            FeatureStability::Unstable => Err(GateError::UnstableDisabled { feature }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Edition, EditionError, FeatureGates, FeatureStability, GateError, LanguageVersion,
        VersionError,
    };

    #[test]
    fn version_parsing() {
        assert_eq!(LanguageVersion::parse("0.1"), Ok(LanguageVersion::CURRENT));
        assert_eq!(
            LanguageVersion::parse("12.34"),
            Ok(LanguageVersion::new(12, 34))
        );
        assert_eq!(LanguageVersion::parse("1").unwrap_err().text(), "1");
        assert_eq!(LanguageVersion::parse("1.").unwrap_err().text(), "1.");
        assert_eq!(LanguageVersion::parse(".1").unwrap_err().text(), ".1");
        assert!(matches!(
            LanguageVersion::parse("1.x"),
            Err(VersionError::Malformed { .. })
        ));
        assert_eq!(LanguageVersion::parse("0.1").unwrap().to_string(), "0.1");
    }

    #[test]
    fn version_ordering() {
        assert!(LanguageVersion::new(0, 2) > LanguageVersion::new(0, 1));
        assert!(LanguageVersion::new(1, 0) > LanguageVersion::new(0, 9));
    }

    #[test]
    fn edition_parsing_is_strict_and_names_the_supported_line() {
        assert_eq!(Edition::parse("2026"), Ok(Edition::CURRENT));
        assert_eq!(Edition::CURRENT.to_string(), "2026");
        for rejected in ["2025", "2027", "2026.1", "", "26", "abcd", " 2026"] {
            let error = Edition::parse(rejected).expect_err("the edition is rejected");
            assert!(!error.to_string().is_empty());
        }
        let unsupported = Edition::parse("2027").expect_err("the edition is unsupported");
        assert!(matches!(
            unsupported,
            EditionError::Unsupported { year: 2027 }
        ));
        assert!(unsupported.to_string().contains("2026"));
        assert!(
            Edition::parse("2026.1")
                .unwrap_err()
                .to_string()
                .contains("four-digit")
        );
    }

    #[test]
    fn editions_order_by_year() {
        assert!(Edition::new(2029) > Edition::CURRENT);
        assert_eq!(Edition::CURRENT.year(), 2026);
        assert!(Edition::CURRENT.is_supported());
        assert!(!Edition::new(2027).is_supported());
    }

    #[test]
    fn stable_features_are_always_allowed() {
        let gates = FeatureGates::new(false);
        assert!(
            gates
                .require("stable_thing", FeatureStability::Stable)
                .is_ok()
        );
    }

    #[test]
    fn unstable_features_require_the_flag() {
        let gates = FeatureGates::new(false);
        assert_eq!(
            gates.require("async", FeatureStability::Unstable),
            Err(GateError::UnstableDisabled { feature: "async" })
        );
        assert!(
            gates
                .require("async", FeatureStability::Unstable)
                .unwrap_err()
                .to_string()
                .contains("--allow-unstable-features")
        );
    }

    #[test]
    fn unstable_features_can_be_allowed_globally_or_per_feature() {
        let global = FeatureGates::new(true);
        assert!(global.require("async", FeatureStability::Unstable).is_ok());

        let mut selective = FeatureGates::new(false);
        selective.enable("async");
        assert!(selective.is_enabled("async"));
        assert!(
            selective
                .require("async", FeatureStability::Unstable)
                .is_ok()
        );
        assert!(
            selective
                .require("generators", FeatureStability::Unstable)
                .is_err()
        );
    }
}
