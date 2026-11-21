//! Semantic-version requirements and compatibility-line helpers.

use pubgrub::Ranges;
use semver::{Comparator, Op, Version, VersionReq};

use crate::error::{RegistryError, RegistryResult};

/// A semver compatibility line.
///
/// A line is a major version, or a `0.minor` pair. Two requirements on the
/// same line are solved together; requirements on different lines may select
/// several versions of one package, which is how two majors coexist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Line {
    /// The major version.
    pub major: u64,
    /// The minor version for `0.x` lines.
    pub minor: Option<u64>,
}

impl Line {
    /// Returns the compatibility line of a concrete version.
    pub fn of(version: &Version) -> Self {
        if version.major == 0 {
            Self {
                major: 0,
                minor: Some(version.minor),
            }
        } else {
            Self {
                major: version.major,
                minor: None,
            }
        }
    }

    /// Whether a version belongs to this line.
    pub fn contains(&self, version: &Version) -> bool {
        *self == Self::of(version)
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.minor {
            Some(minor) => write!(f, "{}.{}", self.major, minor),
            None => write!(f, "{}", self.major),
        }
    }
}

/// Parses a requirement with a registry diagnostic.
pub fn parse_requirement(text: &str) -> RegistryResult<VersionReq> {
    VersionReq::parse(text).map_err(|error| {
        RegistryError::resolve(format!(
            "the version requirement `{text}` is invalid: {error}"
        ))
    })
}

/// Converts a requirement to the interval representation used by PubGrub.
///
/// `version-ranges` ignores prerelease identifiers when it builds bounds,
/// which loses `^1.2.0-beta.1` lower bounds. This conversion keeps the exact
/// comparator version, including its prerelease tag.
pub fn range(requirement: &VersionReq) -> Ranges<Version> {
    let mut ranges = Ranges::full();
    for comparator in &requirement.comparators {
        ranges = ranges.intersection(&comparator_range(comparator));
    }
    ranges
}

fn comparator_range(comparator: &Comparator) -> Ranges<Version> {
    let version = ComparatorVersion::of(comparator).version();
    let major = comparator.major;
    let minor = comparator.minor;
    let patch = comparator.patch;
    match comparator.op {
        Op::Exact => match (minor, patch) {
            (None, _) => version_interval(major, 0, 0, major + 1, 0, 0),
            (Some(minor), None) => version_interval(major, minor, 0, major, minor + 1, 0),
            (Some(_), Some(_)) => Ranges::singleton(version),
        },
        Op::Greater => Ranges::strictly_higher_than(version),
        Op::GreaterEq => Ranges::higher_than(version),
        Op::Less => Ranges::strictly_lower_than(version),
        Op::LessEq => match (minor, patch) {
            (None, _) => Ranges::strictly_lower_than(Version::new(major + 1, 0, 0)),
            (Some(minor), None) => Ranges::strictly_lower_than(Version::new(major, minor + 1, 0)),
            (Some(_), Some(_)) => Ranges::lower_than(version),
        },
        Op::Tilde => match (minor, patch) {
            (None, _) => version_interval(major, 0, 0, major + 1, 0, 0),
            (Some(minor), None) => version_interval(major, minor, 0, major, minor + 1, 0),
            (Some(_), Some(_)) => {
                version_interval_versions(version, Version::new(major, minor.unwrap_or(0) + 1, 0))
            }
        },
        Op::Caret => {
            if major > 0 {
                match (minor, patch) {
                    (Some(_), Some(_)) => {
                        version_interval_versions(version, Version::new(major + 1, 0, 0))
                    }
                    (Some(minor), None) => version_interval(major, minor, 0, major + 1, 0, 0),
                    (None, _) => version_interval(major, 0, 0, major + 1, 0, 0),
                }
            } else {
                match (minor, patch) {
                    (Some(0), Some(patch)) => {
                        version_interval_versions(version, Version::new(0, 0, patch + 1))
                    }
                    (Some(0), None) => version_interval(0, 0, 0, 0, 1, 0),
                    (Some(minor), Some(_)) => {
                        version_interval_versions(version, Version::new(0, minor + 1, 0))
                    }
                    (Some(minor), None) => version_interval(0, minor, 0, 0, minor + 1, 0),
                    (None, _) => version_interval(0, 0, 0, 1, 0, 0),
                }
            }
        }
        Op::Wildcard => match minor {
            Some(minor) => version_interval(major, minor, 0, major, minor + 1, 0),
            None => version_interval(major, 0, 0, major + 1, 0, 0),
        },
        _ => Ranges::from_req(VersionReq {
            comparators: vec![comparator.clone()],
        }),
    }
}

fn version_interval(
    major: u64,
    minor: u64,
    patch: u64,
    upper_major: u64,
    upper_minor: u64,
    upper_patch: u64,
) -> Ranges<Version> {
    version_interval_versions(
        Version::new(major, minor, patch),
        Version::new(upper_major, upper_minor, upper_patch),
    )
}

fn version_interval_versions(lower: Version, upper: Version) -> Ranges<Version> {
    Ranges::higher_than(lower).intersection(&Ranges::strictly_lower_than(upper))
}

struct ComparatorVersion<'a>(&'a Comparator);

impl ComparatorVersion<'_> {
    fn of(comparator: &Comparator) -> ComparatorVersion<'_> {
        ComparatorVersion(comparator)
    }

    fn version(&self) -> Version {
        let comparator = self.0;
        let mut version = Version::new(
            comparator.major,
            comparator.minor.unwrap_or(0),
            comparator.patch.unwrap_or(0),
        );
        version.pre = comparator.pre.clone();
        version
    }
}

/// Returns whether a requirement explicitly opts into prereleases.
pub fn allows_prerelease(requirement: &VersionReq) -> bool {
    requirement
        .comparators
        .iter()
        .any(|comparator| !comparator.pre.is_empty())
}

/// Chooses the compatibility line for a requirement.
///
/// An unconstrained `*` selects the newest available major. Every other
/// requirement uses the lowest available version it accepts, which keeps
/// `>=1, <3` on line 1 instead of needlessly splitting it from `^1`.
pub fn line_for(
    requirement: &VersionReq,
    versions: impl IntoIterator<Item = Version>,
) -> Option<Line> {
    let mut versions: Vec<Version> = versions.into_iter().collect();
    versions.sort();
    if is_unconstrained(requirement) {
        return versions
            .iter()
            .rev()
            .find(|version| version.pre.is_empty())
            .or_else(|| versions.last())
            .map(Line::of);
    }
    versions
        .iter()
        .find(|version| requirement.matches(version))
        .map(Line::of)
}

/// Whether every comparator is absent (the `*` requirement).
pub fn is_unconstrained(requirement: &VersionReq) -> bool {
    requirement.comparators.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions() -> Vec<Version> {
        [
            "0.1.0", "0.2.0", "1.0.0", "1.2.0", "1.5.0", "2.0.0", "2.3.0",
        ]
        .into_iter()
        .map(|version| Version::parse(version).unwrap())
        .collect()
    }

    #[test]
    fn caret_and_tilde_ranges_follow_semver() {
        let caret = range(&parse_requirement("^1.2").unwrap());
        assert!(caret.contains(&Version::parse("1.5.0").unwrap()));
        assert!(!caret.contains(&Version::parse("2.0.0").unwrap()));
        let tilde = range(&parse_requirement("~1.2").unwrap());
        assert!(tilde.contains(&Version::parse("1.2.9").unwrap()));
        assert!(!tilde.contains(&Version::parse("1.3.0").unwrap()));
    }

    #[test]
    fn exact_and_comparator_requirements_intersect() {
        let exact = range(&parse_requirement("=1.2.0").unwrap());
        assert!(exact.contains(&Version::parse("1.2.0").unwrap()));
        assert!(!exact.contains(&Version::parse("1.2.1").unwrap()));
        let bounded = range(&parse_requirement(">=1.2, <1.5").unwrap());
        assert!(bounded.contains(&Version::parse("1.2.0").unwrap()));
        assert!(!bounded.contains(&Version::parse("1.5.0").unwrap()));
    }

    #[test]
    fn lines_split_zero_minors_but_not_positive_majors() {
        assert_eq!(
            Line::of(&Version::parse("0.2.9").unwrap()),
            Line {
                major: 0,
                minor: Some(2)
            }
        );
        assert_eq!(
            Line::of(&Version::parse("2.9.9").unwrap()),
            Line {
                major: 2,
                minor: None
            }
        );
        let req = parse_requirement(">=1, <3").unwrap();
        assert_eq!(
            line_for(&req, versions()),
            Some(Line::of(&Version::parse("1.0.0").unwrap()))
        );
    }

    #[test]
    fn star_selects_the_newest_major() {
        let line = line_for(&VersionReq::STAR, versions()).expect("versions");
        assert_eq!(line, Line::of(&Version::parse("2.3.0").unwrap()));
    }

    #[test]
    fn prerelease_requires_an_explicit_comparator() {
        assert!(!allows_prerelease(&parse_requirement("^1.2").unwrap()));
        assert!(allows_prerelease(
            &parse_requirement("^1.2.0-beta.1").unwrap()
        ));
    }

    #[test]
    fn a_prerelease_range_contains_its_prerelease() {
        let requirement = parse_requirement("^1.1.0-beta.1").unwrap();
        let version = Version::parse("1.1.0-beta.1").unwrap();
        assert!(requirement.matches(&version));
        assert!(range(&requirement).contains(&version));
        assert_eq!(
            line_for(&requirement, [version.clone()]),
            Some(Line::of(&version))
        );
    }
}
