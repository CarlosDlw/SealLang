//! `seal semver-check`: classify the API change against a baseline.

use std::path::Path;
use std::process::ExitCode;

use seal_api::ApiSnapshot;

use crate::cli::{Cli, SemverCheckArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &SemverCheckArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let current = match crate::cli::snapshot::generate(cli, &project, args.package.as_deref()) {
        Ok(snapshot) => snapshot,
        Err(code) => return code,
    };
    let baseline = match baseline(cli, &project, args, &current) {
        Ok(snapshot) => snapshot,
        Err(code) => return code,
    };
    let diff = match seal_api::check(&baseline, &current) {
        Ok(diff) => diff,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.json {
        match serde_json::to_string_pretty(&diff) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8091",
                    &format!("cannot serialize the compatibility report: {error}"),
                );
            }
        }
    } else {
        println!(
            "{}: {} breaking, {} minor, {} patch change(s); required bump: {}",
            current.package,
            diff.breaking.len(),
            diff.minor.len(),
            diff.patch.len(),
            diff.required_bump().as_str()
        );
        for change in diff.breaking.iter().chain(&diff.minor).chain(&diff.patch) {
            println!("  {} {}: {}", change.kind, change.target, change.detail);
        }
    }
    if diff.breaking.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn baseline(
    cli: &Cli,
    project: &seal_project::Project,
    args: &SemverCheckArgs,
    current: &ApiSnapshot,
) -> Result<ApiSnapshot, ExitCode> {
    let path = Path::new(&args.baseline);
    if path.is_file() {
        return ApiSnapshot::read(path).map_err(|error| ui::error(error.code(), error.message()));
    }
    let version = semver::Version::parse(&args.baseline).map_err(|error| {
        ui::error(
            "E8090",
            &format!(
                "`{}` is not an API snapshot file or a semantic version: {error}",
                args.baseline
            ),
        )
    })?;
    let session = match seal_registry::RegistrySession::open(
        &project.root,
        project.registry.as_ref(),
        cli.offline,
    ) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Err(ui::error(
                "E8020",
                "a version baseline needs `[registry]` or `SEAL_REGISTRY_URL`",
            ));
        }
        Err(error) => return Err(ui::error(error.code(), error.message())),
    };
    let index = session
        .info(&current.package)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    let entry = index.version(&version).ok_or_else(|| {
        ui::error(
            "E8090",
            &format!("`{}` {} is not published", current.package, version),
        )
    })?;
    let api = entry.api.as_ref().ok_or_else(|| {
        ui::error(
            "E8090",
            &format!(
                "`{}` {} has no API snapshot; publish it with `seal publish`",
                current.package, version
            ),
        )
    })?;
    let bytes = session
        .fetch(&api.snapshot)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    let snapshot =
        ApiSnapshot::parse(&bytes).map_err(|error| ui::error(error.code(), error.message()))?;
    if snapshot.hash().ok().as_deref() != Some(api.hash.as_str()) {
        return Err(ui::error(
            "E8090",
            "the published API snapshot does not match its index digest",
        ));
    }
    Ok(snapshot)
}
