//! `seal publish`: build, sign, and submit packages.

mod archive;
mod sbom;
mod signing;

use std::path::Path;
use std::process::ExitCode;

use seal_api::ApiSnapshot;
use seal_project::Project;
use seal_registry::RegistrySession;
use seal_registry::publish::{PublishResponse, publish as send};

use crate::cli::{Cli, PublishArgs};
use crate::ui;

use archive::Bundle;

pub(crate) fn run(cli: &Cli, root: &Path, args: &PublishArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let indexes = match select(&project, args) {
        Ok(indexes) => indexes,
        Err(code) => return code,
    };
    let seed = match signing::load(args.signing_key.as_deref()) {
        Ok(seed) => seed,
        Err(code) => return code,
    };
    let session = match RegistrySession::open(&project.root, project.registry.as_ref(), cli.offline)
    {
        Ok(Some(session)) => Some(session),
        Ok(None) if args.dry_run => None,
        Ok(None) => {
            return ui::error(
                "E8020",
                "the project has no registry; configure `[registry]` or `SEAL_REGISTRY_URL`",
            );
        }
        Err(error) => {
            if args.dry_run {
                ui::warning(&format!(
                    "the registry is unavailable; the dry run will not show an API baseline: {}",
                    error.message()
                ));
                None
            } else {
                return ui::error(error.code(), error.message());
            }
        }
    };
    for index in indexes {
        let package = project.package(index);
        if !args.dry_run && !args.allow_dirty && archive::dirty(&package.root) {
            return ui::error(
                "E8095",
                &format!(
                    "the package `{}` has uncommitted changes; commit them or pass `--allow-dirty`",
                    package.name
                ),
            );
        }
        let snapshot = match crate::cli::snapshot::generate(cli, &project, Some(&package.name)) {
            Ok(snapshot) => snapshot,
            Err(code) => return code,
        };
        let bundle =
            match archive::build(&project, index, &snapshot, &seed, args.identity.as_deref()) {
                Ok(bundle) => bundle,
                Err(failure) => return failure.exit(),
            };
        if args.dry_run {
            dry_run(cli, session.as_ref(), &bundle, &snapshot);
            continue;
        }
        let Some(session) = session.as_ref() else {
            return ui::error("E8020", "a real publish needs a registry");
        };
        if let Some(baseline) = baseline(session, &package.name, &package.version) {
            let Ok(version) = package.version.parse::<semver::Version>() else {
                return ui::error(
                    "E8092",
                    &format!("the package `{}` has an invalid version", package.name),
                );
            };
            if let Err(error) = seal_api::enforce(&baseline, &snapshot, &version) {
                return ui::error(error.code(), error.message());
            }
        }
        ui::status(
            cli,
            &format!(
                "Publishing {} {} ({})",
                package.name, package.version, bundle.checksum
            ),
        );
        match send(session, &bundle.request) {
            Ok(response) => {
                if let Err(failure) = verify(session, &bundle, &response) {
                    return failure.exit();
                }
                ui::detail(
                    cli,
                    &format!(
                        "  log entry {} in tree {} ({})",
                        response.log.index, response.log.tree_size, response.log.root_hash
                    ),
                );
            }
            Err(error) if args.workspace && error.code() == "E8082" => {
                ui::warning(&format!(
                    "{} {} is already published; continuing",
                    package.name, package.version
                ));
            }
            Err(error) => {
                ui::error(
                    error.code(),
                    &format!(
                        "failed to publish `{}` {}; packages published before this point remain: {}",
                        package.name,
                        package.version,
                        error.message()
                    ),
                );
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

fn select(project: &Project, args: &PublishArgs) -> Result<Vec<usize>, ExitCode> {
    if args.workspace {
        return Ok((0..project.packages.len()).collect());
    }
    if let Some(name) = &args.package {
        return project
            .package_by_name(name)
            .map(|(index, _)| vec![index])
            .ok_or_else(|| {
                ui::error(
                    "E8005",
                    &format!("the project has no package named `{name}`"),
                )
            });
    }
    project
        .default_package()
        .map(|index| vec![index])
        .map_err(|error| ui::error(error.code(), error.message()))
}

fn baseline(session: &RegistrySession, name: &str, version: &str) -> Option<ApiSnapshot> {
    let target = semver::Version::parse(version).ok()?;
    let index = session.info(name).ok()?;
    let previous = index
        .parsed_versions()
        .into_iter()
        .filter(|(candidate, entry)| *candidate < target && entry.api.is_some())
        .max_by(|left, right| left.0.cmp(&right.0))?;
    let api = previous.1.api.as_ref()?;
    let bytes = session.fetch(&api.snapshot).ok()?;
    let snapshot = ApiSnapshot::parse(&bytes).ok()?;
    if snapshot.hash().ok().as_deref() != Some(api.hash.as_str()) {
        return None;
    }
    Some(snapshot)
}

fn dry_run(cli: &Cli, session: Option<&RegistrySession>, bundle: &Bundle, snapshot: &ApiSnapshot) {
    ui::status(
        cli,
        &format!(
            "Dry run: {} {} would publish {} file(s), {} bytes compressed",
            bundle.request.name,
            bundle.request.version,
            bundle.files.len(),
            bundle.archive_size
        ),
    );
    for (path, size) in &bundle.files {
        ui::detail(cli, &format!("  {path} ({size} bytes)"));
    }
    ui::detail(
        cli,
        &format!(
            "  archive {}; API {}; SBOM {}; publisher {}",
            bundle.checksum, bundle.api_digest, bundle.sbom_digest, bundle.request.publisher.keyid
        ),
    );
    if let Some(session) = session {
        if let Some(previous) = baseline(session, &bundle.request.name, &bundle.request.version) {
            match seal_api::check(&previous, snapshot) {
                Ok(diff) => {
                    ui::detail(
                        cli,
                        &format!(
                            "  API diff: {} breaking, {} minor; required bump {}",
                            diff.breaking.len(),
                            diff.minor.len(),
                            diff.required_bump().as_str()
                        ),
                    );
                    for change in &diff.breaking {
                        ui::detail(cli, &format!("    {} {}", change.kind, change.target));
                    }
                }
                Err(error) => ui::warning(&format!("cannot classify the API change: {error}")),
            }
        } else {
            ui::detail(cli, "  API diff: no previous published snapshot");
        }
    }
}

fn verify(
    session: &RegistrySession,
    bundle: &Bundle,
    response: &PublishResponse,
) -> Result<(), ui::Failure> {
    if response.checksum != bundle.checksum {
        return Err(ui::failure(
            "E8021",
            format!(
                "the registry stored `{}`, but the archive is `{}`",
                response.checksum, bundle.checksum
            ),
        ));
    }
    if response.api != bundle.api_digest
        || response.sbom != bundle.sbom_digest
        || response.provenance != bundle.provenance_digest
    {
        return Err(ui::failure(
            "E8021",
            "the registry response disagrees with the signed publication",
        ));
    }
    let public = session
        .tuf()
        .log_public_key()
        .map_err(|error| ui::failure(error.code(), error.message()))?;
    seal_registry::log::verify_tree_head(
        &response.tree_head,
        &public,
        Some(response.log.tree_size),
        Some(&response.log.root_hash),
    )
    .map_err(|error| ui::failure(error.code(), error.message()))?;
    seal_registry::log::verify_inclusion(
        &response.log.entry,
        &response.log.proof,
        &response.log.root_hash,
        response.log.tree_size,
    )
    .map_err(|error| ui::failure(error.code(), error.message()))?;
    Ok(())
}
