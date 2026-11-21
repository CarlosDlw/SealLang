//! `seal sbom`: emit a CycloneDX document from the resolved project.

use std::path::Path;
use std::process::ExitCode;

use seal_registry::sbom::{Component, SbomInput, generate};

use crate::cli::{Cli, SbomArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &SbomArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let mut components = Vec::new();
    for package in &project.packages {
        let source = package.source.lock_source(
            &project.root,
            &package.root,
            &package.name,
            &package.version,
        );
        let Some(lock) =
            project
                .lockfile
                .package_with_source(&package.name, &package.version, &source)
        else {
            return ui::error(
                "E8031",
                &format!("the lockfile has no entry for `{}`", package.name),
            );
        };
        components.push(Component {
            name: package.name.clone(),
            version: package.version.clone(),
            source,
            checksum: lock.checksum.clone(),
            license: package
                .manifest
                .package
                .as_ref()
                .and_then(|manifest| manifest.license.clone()),
            features: lock.features.clone(),
            targets: package
                .targets
                .iter()
                .map(|target| target.name.clone())
                .collect(),
        });
    }
    let project_version = project
        .root_package()
        .map(|index| project.package(index).version.clone())
        .or_else(|| {
            project
                .packages
                .first()
                .map(|package| package.version.clone())
        })
        .unwrap_or_else(|| "0.0.0".to_owned());
    let document = match generate(&SbomInput {
        project: project.name.clone(),
        project_version,
        toolchain: seal_toolchain::EMBEDDED_VERSION.to_owned(),
        components,
    }) {
        Ok(document) => document,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let text = match serde_json::to_string_pretty(&document) {
        Ok(text) => text,
        Err(error) => return ui::error("E8031", &format!("cannot serialize the SBOM: {error}")),
    };
    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                return ui::error(
                    "E8031",
                    &format!("cannot create `{}`: {error}", parent.display()),
                );
            }
            if let Err(error) = std::fs::write(path, format!("{text}\n")) {
                return ui::error(
                    "E8031",
                    &format!("cannot write `{}`: {error}", path.display()),
                );
            }
            ui::status(cli, &format!("Wrote SBOM to {}", path.display()));
        }
        None => println!("{text}"),
    }
    ExitCode::SUCCESS
}
