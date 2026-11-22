//! Standard-library selection for project compile plans.
//!
//! The project layer owns the policy: a manifest may select `std`, `core`, or
//! `none`; otherwise a hosted target receives `std` and a target with
//! `runtime = "none"` receives `core`. The selected library is loaded and
//! validated once per plan, and every package records its mode and prelude as
//! data the driver resolves against the local installation.

use seal_manifest::{PackageManifest, StdlibMode};
use seal_plan::{PlanPrelude, PlanStdlib, PlanStdlibMode};

use crate::Package;
use crate::error::{ProjectError, ProjectResult};

/// Resolves the effective mode for one package and target.
pub(crate) fn mode(package: &Package, freestanding: bool) -> ProjectResult<PlanStdlibMode> {
    let manifest = package.manifest.package.as_ref().ok_or_else(|| {
        ProjectError::manifest(format!(
            "the package `{}` has no `[package]` table",
            package.name
        ))
    })?;
    match manifest.stdlib {
        Some(StdlibMode::Std) if freestanding => Err(ProjectError::new(
            "E8138",
            format!(
                "the package `{}` selects `stdlib = \"std\"`, but the target is freestanding; use `core` or `none`",
                package.name
            ),
        )),
        Some(StdlibMode::Std) => Ok(PlanStdlibMode::Std),
        Some(StdlibMode::Core) => Ok(PlanStdlibMode::Core),
        Some(StdlibMode::None) => Ok(PlanStdlibMode::None),
        None if freestanding => Ok(PlanStdlibMode::Core),
        None => Ok(PlanStdlibMode::Std),
    }
}

/// A validated standard library and the plan metadata that identifies it.
pub(crate) struct Loaded {
    /// The plan metadata; the plan carries no absolute path.
    pub(crate) info: PlanStdlib,
    /// The loaded library used to resolve prelude module paths.
    pub(crate) library: seal_stdlib::Stdlib,
}

/// Loads the standard library named by the selected modes.
///
/// Returns `None` when every package disables the standard library, so a
/// bootstrap or `none`-only project does not need an installation.
pub(crate) fn load(modes: &[PlanStdlibMode]) -> ProjectResult<Option<Loaded>> {
    if modes.iter().all(|mode| *mode == PlanStdlibMode::None) {
        return Ok(None);
    }
    let library = seal_stdlib::Stdlib::load()
        .map_err(|error| ProjectError::new(error.code(), error.message().to_owned()))?;
    Ok(Some(Loaded {
        info: PlanStdlib {
            language: library.language().to_owned(),
            version: library.version().to_owned(),
            digest: library.digest().to_owned(),
        },
        library,
    }))
}

/// Builds the prelude for one package.
///
/// `no_prelude` and `implicit-prelude = false` both return `None`; the package
/// still receives the standard library as a dependency.
pub(crate) fn prelude(
    stdlib: &seal_stdlib::Stdlib,
    manifest: &PackageManifest,
    mode: PlanStdlibMode,
    no_prelude: bool,
) -> ProjectResult<Option<PlanPrelude>> {
    if no_prelude || !manifest.implicit_prelude || mode == PlanStdlibMode::None {
        return Ok(None);
    }
    let package = match mode {
        PlanStdlibMode::Std => "std",
        PlanStdlibMode::Core => "core",
        PlanStdlibMode::None => return Ok(None),
    };
    let path = stdlib.prelude_for(package).ok_or_else(|| {
        ProjectError::new(
            "E8135",
            format!("the standard library declares no prelude for `{package}`"),
        )
    })?;
    let (source_package, module) = path.split_once("::").ok_or_else(|| {
        ProjectError::new(
            "E8135",
            format!("the standard-library prelude `{path}` is not a `package::module` path"),
        )
    })?;
    if source_package.is_empty() || module.is_empty() || module.split("::").any(str::is_empty) {
        return Err(ProjectError::new(
            "E8135",
            format!("the standard-library prelude `{path}` has an empty segment"),
        ));
    }
    Ok(Some(PlanPrelude {
        package: None,
        stdlib: Some(source_package.to_owned()),
        module: module.to_owned(),
    }))
}
