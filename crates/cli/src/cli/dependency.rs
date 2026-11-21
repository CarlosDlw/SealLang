//! Shared manifest editing and lockfile-diff helpers for dependency commands.

use std::path::{Path, PathBuf};

use seal_project::{Lockfile, Project};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

/// A parsed `name@requirement` command-line dependency.
pub(crate) struct ParsedSpec {
    /// The local manifest alias.
    pub(crate) alias: String,
    /// The registry package name.
    pub(crate) package: String,
    /// The version requirement.
    pub(crate) requirement: String,
}

/// Splits `foo@1.2`, `foo`, or `@scope/foo@^2` into alias, package, and
/// requirement.
pub(crate) fn parse_spec(text: &str) -> Result<ParsedSpec, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the dependency name cannot be empty".to_owned());
    }
    let (name, requirement) = if let Some(rest) = text.strip_prefix('@') {
        if let Some(index) = rest.find('@') {
            (format!("@{}", &rest[..index]), rest[index + 1..].to_owned())
        } else {
            (format!("@{rest}"), "*".to_owned())
        }
    } else if let Some((name, requirement)) = text.split_once('@') {
        (name.to_owned(), requirement.to_owned())
    } else {
        (text.to_owned(), "*".to_owned())
    };
    if name.is_empty()
        || name == "@"
        || name.contains(char::is_whitespace)
        || name.contains("..")
        || requirement.contains(char::is_whitespace)
    {
        return Err(format!(
            "`{text}` is not a package name with an optional version"
        ));
    }
    let alias = name.rsplit('/').next().unwrap_or(&name).to_owned();
    if alias.is_empty() {
        return Err(format!("`{text}` has no package name"));
    }
    if requirement.is_empty() {
        return Err(format!("`{text}` has an empty version requirement"));
    }
    Ok(ParsedSpec {
        alias,
        package: name,
        requirement,
    })
}

/// Returns the manifest of the selected package.
pub(crate) fn manifest_path(project: &Project, package: Option<&str>) -> Result<PathBuf, String> {
    let index = if let Some(name) = package {
        project
            .package_by_name(name)
            .map(|(index, _)| index)
            .ok_or_else(|| format!("the project has no package named `{name}`"))?
    } else {
        project
            .default_package()
            .map_err(|error| error.message().to_owned())?
    };
    let package = project.package(index);
    if !matches!(package.source, seal_project::PackageSource::Path) {
        return Err(format!(
            "the package `{}` is not a local package and cannot be edited",
            package.name
        ));
    }
    Ok(package.root.join("seal.toml"))
}

/// Adds or replaces one dependency in a manifest.
pub(crate) fn add_dependency(
    path: &Path,
    spec: &ParsedSpec,
    features: &[String],
    optional: bool,
    default_features: bool,
) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let mut document: DocumentMut = text
        .parse()
        .map_err(|error| format!("cannot parse `{}`: {error}", path.display()))?;
    let dependencies = document
        .entry("dependencies")
        .or_insert(Item::Table(Table::new()));
    let table = dependencies
        .as_table_mut()
        .ok_or_else(|| "`[dependencies]` is not a table".to_owned())?;
    let mut dependency = InlineTable::new();
    dependency.insert("version", Value::from(spec.requirement.clone()));
    if spec.package != spec.alias {
        dependency.insert("package", Value::from(spec.package.clone()));
    }
    if !features.is_empty() {
        let mut array = Array::new();
        for feature in features {
            array.push(feature.clone());
        }
        dependency.insert("features", Value::Array(array));
    }
    if !default_features {
        dependency.insert("default-features", Value::from(false));
    }
    if optional {
        dependency.insert("optional", Value::from(true));
    }
    table.insert(&spec.alias, Item::Value(Value::InlineTable(dependency)));
    Ok(document.to_string())
}

/// Removes one dependency and returns the new manifest text.
pub(crate) fn remove_dependency(path: &Path, alias: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
    let mut document: DocumentMut = text
        .parse()
        .map_err(|error| format!("cannot parse `{}`: {error}", path.display()))?;
    let table = document
        .get_mut("dependencies")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("`{}` has no `[dependencies]` table", path.display()))?;
    if table.remove(alias).is_none() {
        return Err(format!("the manifest has no dependency named `{alias}`"));
    }
    Ok(document.to_string())
}

/// Prints the packages that changed between two lockfiles.
pub(crate) fn print_lock_diff(old: Option<&Lockfile>, new: &Lockfile) {
    let old = old.cloned().unwrap_or_default();
    let old_packages: std::collections::BTreeSet<(String, String, String)> = old
        .packages
        .iter()
        .map(|package| {
            (
                package.name.clone(),
                package.version.clone(),
                package.source.clone(),
            )
        })
        .collect();
    let new_packages: std::collections::BTreeSet<(String, String, String)> = new
        .packages
        .iter()
        .map(|package| {
            (
                package.name.clone(),
                package.version.clone(),
                package.source.clone(),
            )
        })
        .collect();
    for package in old_packages.difference(&new_packages) {
        println!("- {} {} ({})", package.0, package.1, package.2);
    }
    for package in new_packages.difference(&old_packages) {
        println!("+ {} {} ({})", package.0, package.1, package.2);
    }
}
