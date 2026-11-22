//! `seal vendor`: copy verified registry packages into the project.

use std::path::Path;
use std::process::ExitCode;

use seal_project::{PackageSource, Project, ProjectOptions};
use seal_registry::vendor::{self, VendorInput};

use crate::cli::{Cli, VendorArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &VendorArgs) -> ExitCode {
    let options = ProjectOptions {
        offline: cli.offline,
        minimal_versions: cli.minimal_versions,
        vendor_only: false,
        ..ProjectOptions::default()
    };
    let project = match Project::load_with(root, &options) {
        Ok(project) => project,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.clean {
        let directory = root.join("vendor");
        if directory.exists()
            && let Err(error) = std::fs::remove_dir_all(&directory)
        {
            return ui::error(
                "E8029",
                &format!("cannot remove `{}`: {error}", directory.display()),
            );
        }
    }
    let mut inputs = Vec::new();
    for package in &project.packages {
        let PackageSource::Registry(source) = &package.source else {
            continue;
        };
        let version = match semver::Version::parse(&package.version) {
            Ok(version) => version,
            Err(error) => {
                return ui::error(
                    "E8029",
                    &format!(
                        "the package `{}` has the invalid version `{}`: {error}",
                        package.name, package.version
                    ),
                );
            }
        };
        inputs.push(VendorInput {
            name: package.name.clone(),
            version,
            checksum: source.checksum.clone(),
            source: package.source.lock_source(
                root,
                &package.root,
                &package.name,
                &package.version,
            ),
            root: package.root.clone(),
        });
    }
    match vendor::install(root, &inputs) {
        Ok(manifest) => {
            if let Err(error) = project.write_lockfile() {
                return ui::error(error.code(), error.message());
            }
            ui::status(
                cli,
                &format!(
                    "Vendored {} package(s) into vendor/",
                    manifest.package.len()
                ),
            );
        }
        Err(error) => return ui::error(error.code(), error.message()),
    }
    let verify = ProjectOptions {
        offline: true,
        vendor_only: true,
        ..options
    };
    match Project::load_with(root, &verify) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => ui::error(
            error.code(),
            &format!(
                "the new vendor tree does not load offline: {}",
                error.message()
            ),
        ),
    }
}
