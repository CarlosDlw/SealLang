//! `seal audit` and `seal deny`: dependency policy and advisory checks.

use std::path::Path;
use std::process::ExitCode;

use seal_project::PackageSource;
use seal_registry::RegistrySession;
use seal_registry::audit::{AuditInput, audit};

use crate::cli::{AuditArgs, Cli};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &AuditArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let mut inputs = Vec::new();
    for package in &project.packages {
        let PackageSource::Registry(source) = &package.source else {
            continue;
        };
        let version = match semver::Version::parse(&package.version) {
            Ok(version) => version,
            Err(error) => {
                return ui::error(
                    "E8030",
                    &format!(
                        "the package `{}` has the invalid version `{}`: {error}",
                        package.name, package.version
                    ),
                );
            }
        };
        inputs.push(AuditInput {
            name: package.name.clone(),
            version,
            source: package.source.lock_source(
                root,
                &package.root,
                &package.name,
                &package.version,
            ),
            license: package
                .manifest
                .package
                .as_ref()
                .and_then(|manifest| manifest.license.clone()),
            checksum: source.checksum.clone(),
            root: package.root.clone(),
            yanked: source.yanked,
            provenance: source.provenance.clone(),
        });
    }
    let advisories = if inputs.is_empty() {
        Vec::new()
    } else {
        let session =
            match RegistrySession::open(&project.root, project.registry.as_ref(), cli.offline) {
                Ok(Some(session)) => session,
                Ok(None) => {
                    return ui::error(
                        "E8020",
                        "registry packages are present, but no registry is configured",
                    );
                }
                Err(error) => return ui::error(error.code(), error.message()),
            };
        match session.advisories() {
            Ok(advisories) => advisories,
            Err(error) => return ui::error(error.code(), error.message()),
        }
    };
    let policy = project
        .registry
        .as_ref()
        .map(|registry| registry.policy.clone())
        .unwrap_or_default();
    let report = match audit(&inputs, &advisories, &policy) {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error("E8030", &format!("cannot serialize the audit: {error}"));
            }
        }
    } else if report.violations.is_empty() {
        println!("audit passed: {} package(s) checked", report.packages);
    } else {
        println!("audit failed: {} finding(s)", report.violations.len());
        for violation in &report.violations {
            println!(
                "{} {}: {}",
                violation.code, violation.package, violation.message
            );
        }
    }
    if report.violations.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
