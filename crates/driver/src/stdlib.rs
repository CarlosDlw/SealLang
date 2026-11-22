//! Standard-library packages as HIR plan entries.
//!
//! The compile plan names the standard library only by language, version, and
//! digest. The driver loads the local installation, verifies that identity, and
//! turns the selected packages into ordinary [`seal_hir::PackagePlanEntry`]
//! values. Their source identities are `<stdlib>/...`, so diagnostics and debug
//! information never contain the installation path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use seal_ast::Ast;
use seal_def::{PackageId, PackageInstanceId};
use seal_hir::{CfgEnvironment, PackagePlanDependency, PackagePlanEntry, PackagePlanFile};
use seal_index::Idx;
use seal_query::Dependency;
use seal_session::Session;
use seal_span::{FileId, FileName};

/// The HIR entries and query inputs contributed by the standard library.
pub(crate) struct Injected {
    /// Packages in dependency order, with indexes starting at zero.
    pub(crate) entries: Vec<PackagePlanEntry>,
    /// Parsed files for the HIR package unit.
    pub(crate) loaded: Vec<(FileId, Ast)>,
    /// Parse dependencies for the HIR package unit.
    pub(crate) dependencies: Vec<Dependency>,
    /// Standard-library package name to its index in `entries`.
    pub(crate) indexes: BTreeMap<String, usize>,
}

/// Returns the standard-library packages one mode requires.
pub(crate) fn needed_for_mode(mode: seal_plan::PlanStdlibMode) -> BTreeSet<String> {
    let mut needed = BTreeSet::new();
    if mode != seal_plan::PlanStdlibMode::None {
        needed.insert("core".to_owned());
    }
    if mode == seal_plan::PlanStdlibMode::Std {
        needed.insert("std".to_owned());
    }
    needed
}

/// Builds the HIR environment for standard-library packages.
pub(crate) fn cfg(session: &Session) -> CfgEnvironment {
    let target = session.target();
    CfgEnvironment {
        features: BTreeSet::new(),
        target_os: target.triple().os().as_str().to_owned(),
        target_arch: target.triple().arch().as_str().to_owned(),
        target_env: String::new(),
        pointer_width: target.pointer_width(),
        debug_assertions: true,
        test: false,
        bench: false,
        env: BTreeMap::new(),
    }
}

/// Loads every package named by `needed` and its internal dependencies.
///
/// `instance_base` is one past the highest user-package instance, so the two
/// namespaces never share a `PackageInstanceId`.
pub(crate) fn load(
    session: &Session,
    stdlib: &seal_stdlib::Stdlib,
    needed: &BTreeSet<String>,
    cfg: CfgEnvironment,
    instance_base: usize,
) -> Result<Injected, String> {
    if needed.is_empty() {
        return Ok(Injected {
            entries: Vec::new(),
            loaded: Vec::new(),
            dependencies: Vec::new(),
            indexes: BTreeMap::new(),
        });
    }

    let mut selected = needed.clone();
    let mut pending: Vec<String> = needed.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        let package = stdlib.package(&name).ok_or_else(|| {
            format!("the standard library has no package `{name}` required by the plan")
        })?;
        for dependency in &package.dependencies {
            if selected.insert(dependency.package.clone()) {
                pending.push(dependency.package.clone());
            }
        }
    }

    let packages: Vec<&seal_stdlib::Package> = stdlib
        .packages()
        .iter()
        .filter(|package| selected.contains(&package.name))
        .collect();
    let indexes: BTreeMap<String, usize> = packages
        .iter()
        .enumerate()
        .map(|(index, package)| (package.name.clone(), index))
        .collect();
    for package in &packages {
        for dependency in &package.dependencies {
            if !indexes.contains_key(&dependency.package) {
                return Err(format!(
                    "the standard-library package `{}` depends on `{}` before it in dependency order",
                    package.name, dependency.package
                ));
            }
        }
    }

    let mut entries = Vec::with_capacity(packages.len());
    let mut loaded = Vec::new();
    let mut dependencies = Vec::new();
    for (position, package) in packages.iter().enumerate() {
        let mut files = Vec::with_capacity(package.files.len());
        let mut primary_assigned = false;
        for file in &package.files {
            let identity = standard_library_identity(stdlib.root(), file)?;
            // The file is read from disk but registered under `<stdlib>/...`,
            // so no diagnostic or debug location can leak the installation
            // path. The virtual name is also the query key for the file.
            let source_text = seal_fs::load(file, seal_fs::DEFAULT_MAX_FILE_SIZE)
                .map_err(|error| format!("cannot load `{identity}`: {error}"))?;
            let file_id = session
                .add_file(FileName::Virtual(identity.clone()), source_text.into_text())
                .map_err(|_| format!("cannot register the standard-library source `{identity}`"))?;
            let source = session
                .source_map()
                .file(file_id)
                .ok_or_else(|| format!("the standard-library source `{identity}` has no text"))?;
            let tokens = crate::frontend::lex(session, file_id, &source)
                .ok_or_else(|| format!("cannot lex the standard-library source `{identity}`"))?;
            let ast = crate::frontend::parse(session, file_id, &source, &tokens)
                .ok_or_else(|| format!("cannot parse the standard-library source `{identity}`"))?;
            let dependency = crate::frontend::ast_dependency(session, file_id, &source)
                .map_err(|error| format!("the parse dependency is missing: {error}"))?;
            dependencies.push(dependency);
            let is_library_root = file.ends_with(Path::new("src").join("lib.sl"));
            let primary = if is_library_root && !primary_assigned {
                primary_assigned = true;
                true
            } else {
                false
            };
            files.push(PackagePlanFile {
                file: file_id,
                primary,
            });
            loaded.push((file_id, Arc::unwrap_or_clone(ast)));
        }
        if !primary_assigned && let Some(first) = files.first_mut() {
            first.primary = true;
        }
        let mut exposed = Vec::with_capacity(package.exposed.len());
        for path in &package.exposed {
            let module = crate::pipeline::module_path(session, path).ok_or_else(|| {
                format!(
                    "the standard-library package `{}` exposes the empty module path `{path}`",
                    package.name
                )
            })?;
            exposed.push(module);
        }
        let mut edges = Vec::with_capacity(package.dependencies.len());
        for dependency in &package.dependencies {
            let index = indexes.get(&dependency.package).ok_or_else(|| {
                format!(
                    "the standard-library dependency `{}` has no injected package",
                    dependency.package
                )
            })?;
            edges.push(PackagePlanDependency {
                alias: session.interner().intern(&dependency.alias),
                package: PackageId::from_usize(*index),
            });
        }
        entries.push(PackagePlanEntry {
            name: session.interner().intern(&package.name),
            instance: PackageInstanceId::from_usize(instance_base + position),
            files,
            exposed,
            dependencies: edges,
            cfg: cfg.clone(),
            prelude: None,
        });
    }

    Ok(Injected {
        entries,
        loaded,
        dependencies,
        indexes,
    })
}

/// Resolves a standard-library package name to its HIR package identity.
pub(crate) fn package_id(injected: &Injected, name: &str) -> Result<PackageId, String> {
    injected
        .indexes
        .get(name)
        .map(|index| PackageId::from_usize(*index))
        .ok_or_else(|| {
            format!(
                "the plan prelude names the standard-library package `{name}`, which is not injected"
            )
        })
}

fn standard_library_identity(root: &Path, file: &Path) -> Result<String, String> {
    let relative = file.strip_prefix(root).map_err(|_| {
        format!(
            "the standard-library source `{}` is outside `{}`",
            file.display(),
            root.display()
        )
    })?;
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!("<stdlib>/{relative}"))
}
