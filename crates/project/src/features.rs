//! Per-edge feature resolution.
//!
//! Features are additive inside one package instance, but never unified
//! globally: two dependents that ask for different sets get two instances of
//! the same package. The resolver also expands `dep:`, `dep/feature`, and
//! `dep?/feature` requests and enforces the declared conflict table and the
//! per-package instance cap.

use std::collections::{BTreeMap, BTreeSet};

use crate::cfg::CfgEnvironment;
use crate::error::{ProjectError, ProjectResult};
use crate::{Package, Project, Target};

/// The maximum number of feature variants kept for one package.
pub(crate) const MAX_FEATURE_VARIANTS: usize = 8;

/// One package instance selected for a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Instance {
    /// The source package index.
    pub(crate) package: usize,
    /// The fully expanded feature set.
    pub(crate) features: BTreeSet<String>,
    /// Active dependency edges, sorted by alias.
    pub(crate) dependencies: Vec<InstanceEdge>,
}

/// One dependency edge between two package instances.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstanceEdge {
    /// The local import alias.
    pub(crate) alias: String,
    /// The index of the dependency declaration on the parent package.
    pub(crate) dependency: usize,
    /// The dependency instance index.
    pub(crate) instance: usize,
    /// The features requested on this edge.
    pub(crate) features: BTreeSet<String>,
}

/// The resolved instance graph for one compilation target.
#[derive(Clone, Debug)]
pub(crate) struct InstanceGraph {
    /// Instances in dependency-first order.
    pub(crate) instances: Vec<Instance>,
    /// The primary package instance.
    pub(crate) primary: usize,
}

/// Resolves the instance graph for `target`.
pub(crate) fn resolve(
    project: &Project,
    package: usize,
    target: &Target,
    options: &crate::PlanOptions,
    cfg: &CfgEnvironment,
) -> ProjectResult<InstanceGraph> {
    let mut builder = Builder {
        project,
        cfg,
        cache: BTreeMap::new(),
        counts: BTreeMap::new(),
        instances: Vec::new(),
    };
    let package_manifest = project.package(package);
    let mut requested = BTreeSet::new();
    if !options.no_default_features
        && package_manifest
            .manifest
            .package
            .as_ref()
            .is_some_and(|p| p.features.contains("default"))
    {
        requested.insert("default".to_owned());
    }
    if options.all_features {
        if let Some(manifest) = &package_manifest.manifest.package {
            requested.extend(manifest.features.names().map(ToOwned::to_owned));
        }
    } else {
        requested.extend(options.features.iter().cloned());
    }
    requested.extend(
        package_manifest
            .target(&target.name)
            .map(|target| target.required_features.clone())
            .unwrap_or_default(),
    );
    let primary = builder.visit(package, requested, false)?;
    let features = &builder.instances[primary].features;
    for required in &target.required_features {
        if !features.contains(required) {
            return Err(ProjectError::feature(format!(
                "the target `{}` requires the feature `{required}`, which was not enabled",
                target.name
            )));
        }
    }
    Ok(InstanceGraph {
        instances: builder.instances,
        primary,
    })
}

struct Builder<'a> {
    project: &'a Project,
    cfg: &'a CfgEnvironment,
    cache: BTreeMap<(usize, Vec<String>), usize>,
    counts: BTreeMap<String, usize>,
    instances: Vec<Instance>,
}

impl Builder<'_> {
    fn visit(
        &mut self,
        package_index: usize,
        requested: BTreeSet<String>,
        include_default: bool,
    ) -> ProjectResult<usize> {
        let package = self.project.package(package_index);
        let manifest = package.manifest.package.as_ref().ok_or_else(|| {
            ProjectError::manifest(format!(
                "the package `{}` has no package table",
                package.name
            ))
        })?;

        let mut seeds: Vec<String> = requested.into_iter().collect();
        if include_default && manifest.features.contains("default") {
            seeds.push("default".to_owned());
        }
        let mut expansion = Expansion::default();
        for seed in &seeds {
            expansion.apply(package, seed)?;
        }
        expansion.close(package)?;
        expansion.check_conflicts(package)?;

        let mut key: Vec<String> = expansion.features.iter().cloned().collect();
        key.sort();
        if let Some(&index) = self.cache.get(&(package_index, key.clone())) {
            return Ok(index);
        }
        let count = self.counts.entry(package.name.clone()).or_default();
        *count += 1;
        if *count > MAX_FEATURE_VARIANTS {
            return Err(ProjectError::feature(format!(
                "the package `{}` would have {count} feature variants; the limit is {MAX_FEATURE_VARIANTS}; align the features requested by its dependents",
                package.name
            )));
        }

        let mut dependencies = Vec::new();
        for (dependency_index, dependency) in package.dependencies.iter().enumerate() {
            if let Some(condition) = &dependency.when {
                if !self.cfg.matches(condition) {
                    continue;
                }
            }
            if dependency.optional && !expansion.active_dependencies.contains(&dependency.alias) {
                continue;
            }
            let mut requested = dependency.features.iter().cloned().collect::<BTreeSet<_>>();
            if let Some(extra) = expansion.dependency_features.get(&dependency.alias) {
                requested.extend(extra.iter().cloned());
            }
            let edge_features = requested.clone();
            let instance =
                self.visit(dependency.package, requested, dependency.default_features)?;
            dependencies.push(InstanceEdge {
                alias: dependency.alias.clone(),
                dependency: dependency_index,
                instance,
                features: edge_features,
            });
        }
        dependencies.sort_by(|left, right| left.alias.cmp(&right.alias));

        let features = expansion.features;
        let index = self.instances.len();
        self.instances.push(Instance {
            package: package_index,
            features: features.clone(),
            dependencies,
        });
        self.cache.insert((package_index, key), index);
        Ok(index)
    }
}

#[derive(Default)]
struct Expansion {
    features: BTreeSet<String>,
    active_dependencies: BTreeSet<String>,
    dependency_features: BTreeMap<String, BTreeSet<String>>,
}

impl Expansion {
    /// Applies one feature request, expanding nested feature definitions.
    fn apply(&mut self, package: &Package, request: &str) -> ProjectResult<()> {
        if let Some(alias) = request.strip_prefix("dep:") {
            validate_alias(package, alias)?;
            self.active_dependencies.insert(alias.to_owned());
            return Ok(());
        }
        if let Some((dependency, feature)) = request.split_once('/') {
            let weak = dependency.ends_with('?');
            let alias = dependency.strip_suffix('?').unwrap_or(dependency);
            let dependency = validate_alias(package, alias)?;
            let active = if dependency.optional {
                self.active_dependencies.contains(alias)
            } else {
                true
            };
            if !weak {
                self.active_dependencies.insert(alias.to_owned());
            }
            if active || !weak {
                self.dependency_features
                    .entry(alias.to_owned())
                    .or_default()
                    .insert(feature.to_owned());
            }
            return Ok(());
        }
        let manifest = package
            .manifest
            .package
            .as_ref()
            .ok_or_else(|| ProjectError::manifest("a package has no package table"))?;
        if !manifest.features.contains(request) {
            return Err(ProjectError::feature(format!(
                "the package `{}` has no feature named `{request}`",
                package.name
            )));
        }
        self.features.insert(request.to_owned());
        Ok(())
    }

    /// Expands feature values until the set reaches a fixed point.
    fn close(&mut self, package: &Package) -> ProjectResult<()> {
        let manifest = package
            .manifest
            .package
            .as_ref()
            .ok_or_else(|| ProjectError::manifest("a package has no package table"))?;
        loop {
            let before = self.features.len();
            let current: Vec<String> = self.features.iter().cloned().collect();
            for feature in current {
                let Some(values) = manifest.features.get(&feature) else {
                    continue;
                };
                for value in values {
                    self.apply(package, value)?;
                }
            }
            if self.features.len() == before {
                return Ok(());
            }
        }
    }

    fn check_conflicts(&self, package: &Package) -> ProjectResult<()> {
        let Some(manifest) = &package.manifest.package else {
            return Ok(());
        };
        for (feature, conflicts) in manifest.features.conflict_map() {
            if !self.features.contains(feature) {
                continue;
            }
            if let Some(other) = conflicts
                .iter()
                .find(|other| self.features.contains(other.as_str()))
            {
                return Err(ProjectError::feature(format!(
                    "the features `{feature}` and `{other}` of package `{}` conflict",
                    package.name
                )));
            }
        }
        Ok(())
    }
}

fn validate_alias<'a>(package: &'a Package, alias: &str) -> ProjectResult<&'a crate::Dependency> {
    package
        .dependencies
        .iter()
        .find(|dependency| dependency.alias == alias)
        .ok_or_else(|| {
            ProjectError::feature(format!(
                "the package `{}` has no dependency named `{alias}`",
                package.name
            ))
        })
}
