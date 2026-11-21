//! PubGrub version resolution with compatibility lines.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use pubgrub::{
    DefaultStringReporter, Dependencies, DependencyConstraints, DependencyProvider, Map,
    PackageResolutionStatistics, PubGrubError, Ranges, Reporter, resolve as solve,
};
use semver::{Version, VersionReq};

use crate::client::RegistryClient;
use crate::config::RegistryConfig;
use crate::error::{RegistryError, RegistryResult};
use crate::index::{self, IndexPackage, IndexVersion};
use crate::semver::{Line, allows_prerelease, line_for, parse_requirement};
use crate::tuf::TufSession;

/// A direct registry requirement from a local package.
#[derive(Clone, Debug)]
pub struct Requirement {
    /// The package name.
    pub package: String,
    /// The local alias, used only in diagnostics.
    pub alias: String,
    /// The version requirement.
    pub requirement: VersionReq,
    /// The requiring package, used in diagnostics.
    pub source: String,
}

impl Requirement {
    /// Creates a requirement from its textual form.
    pub fn parse(
        package: impl Into<String>,
        alias: impl Into<String>,
        requirement: &str,
        source: impl Into<String>,
    ) -> RegistryResult<Self> {
        Ok(Self {
            package: package.into(),
            alias: alias.into(),
            requirement: parse_requirement(requirement)?,
            source: source.into(),
        })
    }
}

/// Options that affect version selection.
#[derive(Clone, Debug, Default)]
pub struct ResolveOptions {
    /// Prefer the lowest compatible stable version.
    pub minimal_versions: bool,
    /// Exact versions already locked, by package and compatibility line.
    pub locked: BTreeMap<(String, Line), Version>,
    /// Exact versions requested by `--precise`.
    pub pinned: BTreeMap<String, Version>,
    /// Maximum number of solver loop iterations.
    pub step_limit: usize,
}

/// One selected registry package.
#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    /// The package name.
    pub name: String,
    /// The selected version.
    pub version: Version,
    /// The index entry.
    pub entry: IndexVersion,
}

/// Resolves direct and transitive registry requirements.
pub fn resolve(
    client: &RegistryClient,
    config: &RegistryConfig,
    tuf: Option<&TufSession>,
    requirements: &[Requirement],
    options: &ResolveOptions,
) -> RegistryResult<Vec<ResolvedPackage>> {
    if requirements.is_empty() {
        return Ok(Vec::new());
    }
    let mut prerelease = BTreeSet::new();
    for requirement in requirements {
        if allows_prerelease(&requirement.requirement) {
            prerelease.insert(requirement.package.clone());
        }
    }
    for ((name, _), version) in &options.locked {
        if !version.pre.is_empty() {
            prerelease.insert(name.clone());
        }
    }
    let provider = Provider {
        client,
        config,
        tuf,
        indexes: RefCell::new(BTreeMap::new()),
        minimal_versions: options.minimal_versions,
        locked: options.locked.clone(),
        pinned: options.pinned.clone(),
        requirements: RefCell::new(BTreeMap::new()),
        prerelease: RefCell::new(prerelease),
        steps: Cell::new(0),
        step_limit: if options.step_limit == 0 {
            100_000
        } else {
            options.step_limit
        },
        root_requirements: requirements.to_vec(),
    };
    let solution = match solve(&provider, Package::Root, Version::new(0, 0, 0)) {
        Ok(solution) => solution,
        Err(PubGrubError::NoSolution(mut tree)) => {
            tree.collapse_no_versions();
            let report = DefaultStringReporter::report(&tree);
            return Err(RegistryError::resolve(format!(
                "version resolution failed:\n{report}"
            )));
        }
        Err(PubGrubError::ErrorRetrievingDependencies {
            package,
            version,
            source,
        }) => {
            return Err(RegistryError::resolve(format!(
                "cannot read the dependencies of {package} {version}: {source}"
            )));
        }
        Err(PubGrubError::ErrorChoosingVersion { package, source }) => {
            return Err(RegistryError::resolve(format!(
                "cannot choose a version of {package}: {source}"
            )));
        }
        Err(PubGrubError::ErrorInShouldCancel(error)) => return Err(error),
    };
    let mut packages = Vec::new();
    for (package, version) in solution {
        if let Package::Registry { name, .. } = package {
            let index = provider.index(&name)?;
            let entry = index.version(&version).cloned().ok_or_else(|| {
                RegistryError::resolve(format!(
                    "the index for `{name}` no longer contains {version}"
                ))
            })?;
            packages.push(ResolvedPackage {
                name,
                version,
                entry,
            });
        }
    }
    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    Ok(packages)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Package {
    Root,
    Registry { name: String, line: Line },
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("project"),
            Self::Registry { name, line } => write!(f, "{name} ({line}.x)"),
        }
    }
}

struct Provider<'a> {
    client: &'a RegistryClient,
    config: &'a RegistryConfig,
    tuf: Option<&'a TufSession>,
    indexes: RefCell<BTreeMap<String, IndexPackage>>,
    minimal_versions: bool,
    locked: BTreeMap<(String, Line), Version>,
    pinned: BTreeMap<String, Version>,
    requirements: RefCell<BTreeMap<(String, Line), Vec<VersionReq>>>,
    prerelease: RefCell<BTreeSet<String>>,
    steps: Cell<usize>,
    step_limit: usize,
    root_requirements: Vec<Requirement>,
}

impl Provider<'_> {
    fn index(&self, name: &str) -> RegistryResult<IndexPackage> {
        if let Some(index) = self.indexes.borrow().get(name) {
            return Ok(index.clone());
        }
        let index = index::load(self.client, self.config, self.tuf, name)?;
        self.indexes
            .borrow_mut()
            .insert(name.to_owned(), index.clone());
        Ok(index)
    }

    fn package_line(&self, name: &str, requirement: &VersionReq) -> RegistryResult<Option<Line>> {
        let index = self.index(name)?;
        Ok(line_for(
            requirement,
            index
                .parsed_versions()
                .into_iter()
                .map(|(version, _)| version),
        ))
    }

    fn constraints(
        &self,
        requirements: &[Requirement],
    ) -> RegistryResult<DependencyConstraints<Package, Ranges<Version>>> {
        let mut constraints: DependencyConstraints<Package, Ranges<Version>> = Map::default();
        for requirement in requirements {
            let Some(line) = self.package_line(&requirement.package, &requirement.requirement)?
            else {
                return Err(RegistryError::resolve(format!(
                    "`{}` requires `{}` {}`, but the registry has no matching version",
                    requirement.source, requirement.package, requirement.requirement
                )));
            };
            if allows_prerelease(&requirement.requirement) {
                self.prerelease
                    .borrow_mut()
                    .insert(requirement.package.clone());
            }
            let package = Package::Registry {
                name: requirement.package.clone(),
                line,
            };
            self.requirements
                .borrow_mut()
                .entry((requirement.package.clone(), line))
                .or_default()
                .push(requirement.requirement.clone());
            let range = crate::semver::range(&requirement.requirement);
            if let Some(entry) = constraints.get_mut(&package) {
                *entry = entry.intersection(&range);
            } else {
                constraints.insert(package, range);
            }
        }
        Ok(constraints)
    }

    fn matches_all(&self, name: &str, line: Line, version: &Version) -> bool {
        self.requirements
            .borrow()
            .get(&(name.to_owned(), line))
            .is_none_or(|requirements| {
                requirements
                    .iter()
                    .all(|requirement| requirement.matches(version))
            })
    }

    fn available(
        &self,
        name: &str,
        line: Line,
        range: &Ranges<Version>,
    ) -> RegistryResult<Vec<Version>> {
        let index = self.index(name)?;
        let allow_pre = self.prerelease.borrow().contains(name);
        Ok(index
            .parsed_versions()
            .into_iter()
            .filter(|(version, entry)| {
                Line::of(version) == line
                    && range.contains(version)
                    && self.matches_all(name, line, version)
                    && (allow_pre || version.pre.is_empty())
                    && (!entry.yanked
                        || self
                            .locked
                            .get(&(name.to_owned(), line))
                            .is_some_and(|locked| locked == version))
            })
            .map(|(version, _)| version)
            .collect())
    }
}

impl DependencyProvider for Provider<'_> {
    type P = Package;
    type V = Version;
    type VS = Ranges<Version>;
    type Priority = (u32, Reverse<usize>);
    type M = String;
    type Err = RegistryError;

    fn prioritize(
        &self,
        package: &Self::P,
        range: &Self::VS,
        statistics: &PackageResolutionStatistics,
    ) -> Self::Priority {
        let count = match package {
            Package::Root => 0,
            Package::Registry { name, line } => self
                .available(name, *line, range)
                .map(|versions| versions.len())
                .unwrap_or(0),
        };
        if count == 0 {
            return (u32::MAX, Reverse(0));
        }
        (statistics.conflict_count(), Reverse(count))
    }

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        let Package::Registry { name, line } = package else {
            return Ok(Some(Version::new(0, 0, 0)));
        };
        if let Some(pinned) = self.pinned.get(name) {
            if Line::of(pinned) == *line
                && range.contains(pinned)
                && self.matches_all(name, *line, pinned)
            {
                let index = self.index(name)?;
                if index.version(pinned).is_some_and(|entry| !entry.yanked) {
                    return Ok(Some(pinned.clone()));
                }
            }
        }
        if let Some(locked) = self.locked.get(&(name.clone(), *line)) {
            if range.contains(locked) && self.matches_all(name, *line, locked) {
                let index = self.index(name)?;
                if index.version(locked).is_some() {
                    return Ok(Some(locked.clone()));
                }
            }
        }
        let mut versions = self.available(name, *line, range)?;
        if self.minimal_versions {
            Ok(versions.into_iter().next())
        } else {
            Ok(versions.pop())
        }
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        let requirements = match package {
            Package::Root => self.root_requirements.clone(),
            Package::Registry { name, .. } => {
                let index = self.index(name)?;
                let entry = index
                    .version(version)
                    .ok_or_else(|| {
                        RegistryError::resolve(format!("`{name}` {version} is not in the index"))
                    })?
                    .clone();
                let mut requirements = Vec::new();
                for dependency in &entry.dependencies {
                    let requirement = dependency.requirement()?;
                    requirements.push(Requirement {
                        package: dependency.name.clone(),
                        alias: dependency.alias().to_owned(),
                        requirement,
                        source: format!("{name} {version}"),
                    });
                }
                requirements
            }
        };
        match self.constraints(&requirements) {
            Ok(constraints) => Ok(Dependencies::Available(constraints)),
            Err(error) => Ok(Dependencies::Unavailable(error.to_string())),
        }
    }

    fn should_cancel(&self) -> Result<(), Self::Err> {
        let steps = self.steps.get() + 1;
        self.steps.set(steps);
        if steps > self.step_limit {
            return Err(RegistryError::resolve(format!(
                "version resolution exceeded the {} step limit; simplify the requirements or run `seal update`",
                self.step_limit
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_display_names_the_compatibility_line() {
        let package = Package::Registry {
            name: "foo".to_owned(),
            line: Line {
                major: 1,
                minor: None,
            },
        };
        assert_eq!(package.to_string(), "foo (1.x)");
    }

    #[test]
    fn a_requirement_parses_with_its_source() {
        let requirement = Requirement::parse("foo", "foo", "^1.2", "app 0.1.0").unwrap();
        assert_eq!(requirement.package, "foo");
        assert!(requirement.requirement.matches(&Version::new(1, 5, 0)));
    }
}
