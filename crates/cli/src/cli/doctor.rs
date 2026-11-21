//! `seal doctor`: environment and project health checks.

use std::path::Path;
use std::process::ExitCode;

use seal_project::Project;
use seal_target::host_triple;
use serde::Serialize;

use crate::cli::{Cli, DoctorArgs};
use crate::ui;

#[derive(Serialize)]
struct Report {
    version: u32,
    toolchain: Check,
    linker: Check,
    cache: Check,
    lockfile: Check,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
}

pub(crate) fn doctor(_cli: &Cli, root: &Path, args: &DoctorArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let target = host_triple().as_str().to_owned();
    let toolchain = Check {
        name: "toolchain".to_owned(),
        ok: true,
        detail: format!("seal {} (embedded compiler)", env!("CARGO_PKG_VERSION")),
    };
    let linker = match find_linker() {
        Some(path) => Check {
            name: "linker".to_owned(),
            ok: true,
            detail: path,
        },
        None => Check {
            name: "linker".to_owned(),
            ok: false,
            detail: "no `cc`, `clang`, or `gcc` on PATH".to_owned(),
        },
    };
    let cache = check_cache(&project, args.fix);
    let lockfile = check_lockfile(&project, &target, args.fix);
    let warnings: Vec<String> = project
        .warnings
        .iter()
        .map(|warning| {
            let mut text = warning.message.clone();
            if let Some(suggestion) = &warning.suggestion {
                text.push_str(&format!(" (did you mean `{suggestion}`?)"));
            }
            text
        })
        .collect();
    let report = Report {
        version: 1,
        toolchain,
        linker,
        cache,
        lockfile,
        warnings,
    };
    let ok = report.toolchain.ok && report.linker.ok && report.cache.ok && report.lockfile.ok;
    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error("E8011", &format!("cannot serialize the report: {error}"));
            }
        }
    } else {
        for check in [
            &report.toolchain,
            &report.linker,
            &report.cache,
            &report.lockfile,
        ] {
            println!(
                "{} {}: {}",
                if check.ok { "ok" } else { "FAIL" },
                check.name,
                check.detail
            );
        }
        for warning in &report.warnings {
            println!("warning: {warning}");
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Finds a working C linker on `PATH`.
///
/// The Seal compiler is embedded in this binary, so a missing toolchain is not
/// representable; a missing linker is, and it prevents every executable target
/// from linking.
fn find_linker() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let directories: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    find_linker_in(&directories)
}

fn find_linker_in(directories: &[std::path::PathBuf]) -> Option<String> {
    for directory in directories {
        for candidate in ["cc", "clang", "gcc"] {
            let path = directory.join(candidate);
            if !path.is_file() {
                continue;
            }
            let works = std::process::Command::new(&path)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success());
            if works {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

fn check_cache(project: &Project, fix: bool) -> Check {
    let directory = project.root.join("seal").join("target");
    if fix {
        let _ = std::fs::create_dir_all(&directory);
    }
    let probe = directory.join(".seal-write-test");
    let result = std::fs::create_dir_all(&directory)
        .and_then(|()| std::fs::write(&probe, b"ok"))
        .and_then(|()| std::fs::remove_file(&probe));
    match result {
        Ok(()) => Check {
            name: "cache".to_owned(),
            ok: true,
            detail: directory.display().to_string(),
        },
        Err(error) => Check {
            name: "cache".to_owned(),
            ok: false,
            detail: format!("`{}` is not writable: {error}", directory.display()),
        },
    }
}

fn check_lockfile(project: &Project, target: &str, fix: bool) -> Check {
    let path = project.root.join("seal.lock");
    let resolved = project.resolve_lockfile(target);
    let existing = seal_project::Lockfile::load(&path).ok();
    let expected = match &existing {
        Some(lockfile) => resolved.clone().with_native(lockfile.native.clone()),
        None => resolved.clone(),
    };
    let current = existing
        .as_ref()
        .is_some_and(|lockfile| *lockfile == expected);
    if current {
        return Check {
            name: "lockfile".to_owned(),
            ok: true,
            detail: path.display().to_string(),
        };
    }
    if fix {
        let expected = match &existing {
            Some(lockfile) => resolved.with_native(lockfile.native.clone()),
            None => resolved,
        };
        return match expected.write_if_changed(&path) {
            Ok(_) => Check {
                name: "lockfile".to_owned(),
                ok: true,
                detail: "rewritten".to_owned(),
            },
            Err(error) => Check {
                name: "lockfile".to_owned(),
                ok: false,
                detail: error.message().to_owned(),
            },
        };
    }
    Check {
        name: "lockfile".to_owned(),
        ok: false,
        detail: "missing or stale; run `seal doctor --fix`".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::find_linker_in;

    #[test]
    fn an_empty_search_path_has_no_linker() {
        assert_eq!(find_linker_in(&[]), None);
    }

    #[test]
    fn a_directory_without_a_linker_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("seal-doctor-no-linker-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        assert_eq!(find_linker_in(std::slice::from_ref(&directory)), None);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn a_working_linker_in_the_search_path_is_detected() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        let directory =
            std::env::temp_dir().join(format!("seal-doctor-linker-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let linker = directory.join("cc");
        std::fs::write(&linker, "#!/bin/sh\nexit 0\n").expect("fake linker");
        let mut permissions = std::fs::metadata(&linker).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&linker, permissions).expect("executable permission");

        assert_eq!(
            find_linker_in(&[PathBuf::from(&directory)]),
            Some(linker.display().to_string())
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
