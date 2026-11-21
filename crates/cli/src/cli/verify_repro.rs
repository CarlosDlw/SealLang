//! `seal verify-repro`: build the same project in two directories and compare.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Args;

use crate::cli::{Cli, ui};

/// `seal verify-repro`.
#[derive(Debug, Args)]
pub(crate) struct VerifyReproArgs {
    /// Keep both temporary trees for inspection.
    #[arg(long)]
    pub(crate) keep: bool,
}

/// Builds a copy of the project twice and compares every produced artifact.
pub(crate) fn verify_repro(cli: &Cli, root: &Path, args: &VerifyReproArgs) -> ExitCode {
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return ui::error(
                "E8104",
                &format!("cannot resolve `{}`: {error}", root.display()),
            );
        }
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("seal-repro-{}-{sequence}", std::process::id()));
    let first = base.join("a");
    let second = base.join("b");
    let result = run_comparison(cli, &root, &first, &second);
    if args.keep {
        eprintln!("kept {} and {}", first.display(), second.display());
    } else if let Err(error) = fs::remove_dir_all(&base) {
        eprintln!("warning: cannot remove `{}`: {error}", base.display());
    }
    result
}

fn run_comparison(cli: &Cli, root: &Path, first: &Path, second: &Path) -> ExitCode {
    for copy in [first, second] {
        if let Err(error) = copy_tree(root, copy) {
            return ui::error("E8104", &error);
        }
    }
    for copy in [first, second] {
        if let Err(error) = build_copy(cli, copy) {
            return ui::error("E8104", &error);
        }
    }
    let first_files = match artifacts(first) {
        Ok(files) => files,
        Err(error) => return ui::error("E8104", &error),
    };
    let second_files = match artifacts(second) {
        Ok(files) => files,
        Err(error) => return ui::error("E8104", &error),
    };
    if first_files.keys().ne(second_files.keys()) {
        return ui::error("E8104", "the two builds produced different artifact sets");
    }
    for (name, bytes) in &first_files {
        let other = &second_files[name];
        if bytes != other {
            let position = bytes
                .iter()
                .zip(other)
                .position(|(left, right)| left != right)
                .unwrap_or_else(|| bytes.len().min(other.len()));
            return ui::error(
                "E8104",
                &format!(
                    "artifact `{name}` differs at byte {position} ({} vs {} bytes)",
                    bytes.len(),
                    other.len()
                ),
            );
        }
    }
    ui::status(
        cli,
        &format!(
            "reproducible: {} artifact(s) are byte-identical",
            first_files.len()
        ),
    );
    ExitCode::SUCCESS
}

fn build_copy(cli: &Cli, copy: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot find the seal executable: {error}"))?;
    let output = Command::new(executable)
        .arg("--project")
        .arg(copy)
        .arg("build")
        .output()
        .map_err(|error| format!("cannot build `{}`: {error}", copy.display()))?;
    if !output.status.success() {
        return Err(format!(
            "the build in `{}` failed: {}",
            copy.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if cli.verbose {
        eprintln!("built {}", copy.display());
    }
    Ok(())
}

/// Copies a self-contained project, excluding generated state and VCS data.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create `{}`: {error}", destination.display()))?;
    let entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some("seal" | "target" | ".git")) {
            continue;
        }
        let from = entry.path();
        let to = destination.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect `{}`: {error}", from.display()))?;
        if file_type.is_dir() {
            copy_tree(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)
                .map_err(|error| format!("cannot copy `{}`: {error}", from.display()))?;
        }
    }
    Ok(())
}

/// Reads every regular file below `<project>/seal/target`, keyed by relative path.
fn artifacts(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let target = root.join("seal/target");
    let mut files = BTreeMap::new();
    collect(&target, &target, &mut files)?;
    Ok(files)
}

fn collect(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect `{}`: {error}", path.display()))?;
        if file_type.is_dir() {
            collect(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| format!("`{}` leaves the target directory", path.display()))?;
            let name = relative.to_string_lossy().replace('\\', "/");
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
            files.insert(name, bytes);
        }
    }
    Ok(())
}
