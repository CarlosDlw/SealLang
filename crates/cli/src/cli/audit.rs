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
    let mut session = None;
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
    let advisories = if inputs.is_empty() && !args.require_provenance {
        Vec::new()
    } else {
        let opened = match RegistrySession::open(
            &project.root,
            project.registry.as_ref(),
            cli.offline,
        ) {
            Ok(Some(session)) => session,
            Ok(None) => {
                return ui::error(
                    "E8020",
                    "registry packages or attestations need `[registry]` or `SEAL_REGISTRY_URL`",
                );
            }
            Err(error) => return ui::error(error.code(), error.message()),
        };
        let advisories = match opened.advisories() {
            Ok(advisories) => advisories,
            Err(error) => return ui::error(error.code(), error.message()),
        };
        session = Some(opened);
        advisories
    };
    let mut policy = project
        .registry
        .as_ref()
        .map(|registry| registry.policy.clone())
        .unwrap_or_default();
    if args.require_provenance {
        policy.require_provenance = true;
    }
    let mut report = match audit(&inputs, &advisories, &policy) {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.require_provenance {
        let Some(session) = session.as_ref() else {
            return ui::error("E8020", "attestation checks need a registry");
        };
        for package in &project.packages {
            let PackageSource::Registry(_) = &package.source else {
                continue;
            };
            let Ok(version) = semver::Version::parse(&package.version) else {
                continue;
            };
            let index = match session.info(&package.name) {
                Ok(index) => index,
                Err(error) => return ui::error(error.code(), error.message()),
            };
            let Some(entry) = index.version(&version) else {
                return ui::error(
                    "E8090",
                    &format!("`{}` {} is not in the index", package.name, version),
                );
            };
            if let Err(error) =
                seal_registry::audit::verify_attestations(session, &package.name, &version, entry)
            {
                report.violations.push(seal_registry::audit::Violation {
                    code: error.code().to_owned(),
                    package: format!("{} {}", package.name, package.version),
                    message: error.message().to_owned(),
                });
            }
        }
    }
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
