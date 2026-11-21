//! `seal add`: edit a manifest, resolve, and check the project.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::check::check;
use crate::cli::dependency::{add_dependency, manifest_path, parse_spec, print_lock_diff};
use crate::cli::{AddArgs, CheckArgs, Cli, Selection};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &AddArgs) -> ExitCode {
    if !args.yes && !args.dry_run {
        return ui::error(
            "E8024",
            "`seal add` changes the manifest; pass `--yes` or inspect it with `--dry-run`",
        );
    }
    let spec = match parse_spec(&args.spec) {
        Ok(spec) => spec,
        Err(message) => return ui::error("E8024", &message),
    };
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let path = match manifest_path(&project, args.package.as_deref()) {
        Ok(path) => path,
        Err(message) => return ui::error("E8024", &message),
    };
    let old_manifest = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            return ui::error(
                "E8011",
                &format!("cannot read `{}`: {error}", path.display()),
            );
        }
    };
    let new_manifest = match add_dependency(
        &path,
        &spec,
        &args.features,
        args.optional,
        !args.no_default_features,
    ) {
        Ok(text) => text,
        Err(message) => return ui::error("E8024", &message),
    };
    if let Err(error) = std::fs::write(&path, &new_manifest) {
        return ui::error(
            "E8011",
            &format!("cannot write `{}`: {error}", path.display()),
        );
    }
    let options = crate::cli::project_options(cli);
    let project = match seal_project::Project::load_with(root, &options) {
        Ok(project) => project,
        Err(error) => {
            let _ = std::fs::write(&path, &old_manifest);
            return ui::error(error.code(), error.message());
        }
    };
    if args.dry_run {
        let _ = std::fs::write(&path, &old_manifest);
        println!(
            "would add `{}` with `{}` to `{}`",
            spec.package,
            spec.requirement,
            path.display()
        );
        print_lock_diff(None, &project.lockfile);
        return ExitCode::SUCCESS;
    }
    let old_lock = std::fs::read_to_string(root.join("seal.lock")).ok();
    print_lock_diff(None, &project.lockfile);
    if let Err(error) = project.write_lockfile() {
        restore(&path, &old_manifest, root, old_lock.as_deref());
        return ui::error(error.code(), error.message());
    }
    ui::status(cli, &format!("Added {} {}", spec.package, spec.requirement));
    let code = check(
        cli,
        root,
        &CheckArgs {
            selection: Selection::default_for_command(),
        },
    );
    if code != ExitCode::SUCCESS {
        restore(&path, &old_manifest, root, old_lock.as_deref());
        ui::warning("rolled back the manifest and lockfile because `seal check` failed");
    }
    code
}

fn restore(path: &Path, manifest: &str, root: &Path, lock: Option<&str>) {
    let _ = std::fs::write(path, manifest);
    let lock_path = root.join("seal.lock");
    match lock {
        Some(lock) => {
            let _ = std::fs::write(lock_path, lock);
        }
        None => {
            let _ = std::fs::remove_file(lock_path);
        }
    }
}
