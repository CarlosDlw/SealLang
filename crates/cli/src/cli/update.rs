//! `seal update`: re-resolve registry versions and rewrite the lockfile.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use crate::cli::check::check;
use crate::cli::dependency::{parse_spec, print_lock_diff};
use crate::cli::{CheckArgs, Cli, Selection, UpdateArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &UpdateArgs) -> ExitCode {
    let old_lock = seal_project::Lockfile::load(&root.join("seal.lock")).ok();
    let mut pinned = BTreeMap::new();
    for text in &args.precise {
        let spec = match parse_spec(text) {
            Ok(spec) => spec,
            Err(message) => return ui::error("E8024", &message),
        };
        match semver::Version::parse(&spec.requirement) {
            Ok(version) => {
                pinned.insert(spec.package, version.to_string());
            }
            Err(error) => {
                return ui::error(
                    "E8026",
                    &format!("`--precise {text}` needs an exact version such as `1.2.3`: {error}"),
                );
            }
        }
    }
    let options = seal_project::ProjectOptions {
        offline: cli.offline,
        minimal_versions: cli.minimal_versions,
        update: args.packages.is_empty() && pinned.is_empty(),
        update_packages: args.packages.clone(),
        pinned,
        vendor_only: cli.vendor,
    };
    let project = match seal_project::Project::load_with(root, &options) {
        Ok(project) => project,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    print_lock_diff(old_lock.as_ref(), &project.lockfile);
    if args.dry_run {
        return ExitCode::SUCCESS;
    }
    if let Some(old_lock) = &old_lock
        && *old_lock == project.lockfile
    {
        println!("the lockfile is already up to date");
        return ExitCode::SUCCESS;
    }
    let old_text = std::fs::read_to_string(root.join("seal.lock")).ok();
    if let Err(error) = project.write_lockfile() {
        return ui::error(error.code(), error.message());
    }
    ui::status(cli, "Updated the lockfile");
    let code = check(
        cli,
        root,
        &CheckArgs {
            selection: Selection::default_for_command(),
        },
    );
    if code != ExitCode::SUCCESS {
        match old_text {
            Some(text) => {
                let _ = std::fs::write(root.join("seal.lock"), text);
            }
            None => {
                let _ = std::fs::remove_file(root.join("seal.lock"));
            }
        }
        ui::warning("restored the previous lockfile because `seal check` failed");
    }
    code
}
