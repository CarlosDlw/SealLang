//! Profile inheritance and validation.
//!
//! Only the project root owns profiles. Built-in `dev`, `test`, `release`, and
//! `bench` remain available even when a project declares no table.

use std::collections::BTreeSet;

use seal_manifest::ProfileManifest;
use seal_plan::PlanProfile;

use crate::Project;
use crate::error::{ProjectError, ProjectResult};

/// Resolves a profile name to the values used by the plan.
pub(crate) fn resolve(project: &Project, name: &str) -> ProjectResult<PlanProfile> {
    let mut visited = BTreeSet::new();
    resolve_with(project, name, &mut visited)
}

fn resolve_with(
    project: &Project,
    name: &str,
    visited: &mut BTreeSet<String>,
) -> ProjectResult<PlanProfile> {
    if !visited.insert(name.to_owned()) {
        return Err(ProjectError::profile(format!(
            "the profile `{name}` inherits itself through a cycle"
        )));
    }
    let result = match name {
        "dev" | "test" => {
            let mut profile = PlanProfile::default();
            if let Some(declared) = declared(project, name) {
                apply(&mut profile, declared);
            }
            Ok(profile)
        }
        "release" | "bench" => {
            let mut profile = PlanProfile {
                opt_level: 2,
                debug: false,
                debug_assertions: false,
                lto: "off".to_owned(),
                codegen_units: 1,
                strip: false,
                incremental: false,
            };
            if let Some(declared) = declared(project, name) {
                apply(&mut profile, declared);
            }
            Ok(profile)
        }
        _ => {
            let declared = declared(project, name).ok_or_else(|| {
                ProjectError::profile(format!(
                    "the profile `{name}` is not declared in the project"
                ))
            })?;
            let mut profile = match &declared.inherits {
                Some(parent) => resolve_with(project, parent, visited)?,
                None => PlanProfile::default(),
            };
            apply(&mut profile, declared);
            Ok(profile)
        }
    };
    visited.remove(name);
    result
}

fn declared<'a>(project: &'a Project, name: &str) -> Option<&'a ProfileManifest> {
    project.profiles.get(name)
}

fn apply(profile: &mut PlanProfile, declared: &ProfileManifest) {
    if let Some(value) = declared.opt_level {
        profile.opt_level = value;
    }
    if let Some(value) = declared.debug {
        profile.debug = value;
    }
    if let Some(value) = declared.debug_assertions {
        profile.debug_assertions = value;
    }
    if let Some(value) = &declared.lto {
        profile.lto = value.clone();
    }
    if let Some(value) = declared.codegen_units {
        profile.codegen_units = value;
    }
    if let Some(value) = declared.strip {
        profile.strip = value;
    }
    if let Some(value) = declared.incremental {
        profile.incremental = value;
    }
}
