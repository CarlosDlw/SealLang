//! `seal doctor`: environment and project health checks.

use std::path::Path;
use std::process::ExitCode;

use seal_project::Project;
use seal_sandbox::Sandbox;
use seal_target::host_triple;
use serde::Serialize;

use crate::cli::{Cli, DoctorArgs};
use crate::ui;

#[derive(Serialize)]
struct Report {
    version: u32,
    toolchain: Check,
    target: Check,
    linker: Check,
    sandbox: Check,
    registry: Check,
    manifest: Check,
    cache: Check,
    lockfile: Check,
    disk: Check,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
}

pub(crate) fn doctor(cli: &Cli, root: &Path, args: &DoctorArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let target = host_triple().as_str().to_owned();
    let (toolchain, target_check) = check_toolchain_and_target(&project, &target);
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
    let sandbox = check_sandbox();
    let registry = check_registry(&project);
    let manifest = check_manifest(&project);
    let cache = check_cache(&project, args.fix);
    let lockfile = check_lockfile(&project, &target, args.fix);
    let disk = check_disk(&project, args.fix);
    let mut warnings: Vec<String> = project
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
    if sandbox.ok && sandbox.detail.starts_with("portable") {
        warnings.push("sandbox isolation is portable; generators can read host files".to_owned());
    }
    let report = Report {
        version: 3,
        toolchain,
        target: target_check,
        linker,
        sandbox,
        registry,
        manifest,
        cache,
        lockfile,
        disk,
        warnings,
    };
    let ok = report.toolchain.ok
        && report.target.ok
        && report.linker.ok
        && report.sandbox.ok
        && report.registry.ok
        && report.manifest.ok
        && report.cache.ok
        && report.lockfile.ok
        && report.disk.ok;
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
            &report.target,
            &report.linker,
            &report.sandbox,
            &report.registry,
            &report.manifest,
            &report.cache,
            &report.lockfile,
            &report.disk,
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

fn check_toolchain_and_target(project: &Project, target: &str) -> (Check, Check) {
    let pin = seal_toolchain::read_pin(&project.root);
    let toolchain = match pin {
        Err(error) => Check {
            name: "toolchain".to_owned(),
            ok: false,
            detail: error.to_string(),
        },
        Ok(Some(_pin)) => {
            let store = seal_toolchain::ToolchainStore::for_project(&project.root);
            match store.which(&project.root) {
                Ok(Some(installed)) => match seal_toolchain::ensure_compatible(
                    &installed.manifest,
                    seal_toolchain::EMBEDDED_VERSION,
                    target,
                ) {
                    Ok(()) => Check {
                        name: "toolchain".to_owned(),
                        ok: true,
                        detail: format!("seal {} (pinned, verified)", installed.manifest.version),
                    },
                    Err(error) => Check {
                        name: "toolchain".to_owned(),
                        ok: false,
                        detail: error.to_string(),
                    },
                },
                Ok(None) => Check {
                    name: "toolchain".to_owned(),
                    ok: false,
                    detail: "the project pin is missing".to_owned(),
                },
                Err(error) => Check {
                    name: "toolchain".to_owned(),
                    ok: false,
                    detail: error.to_string(),
                },
            }
        }
        Ok(None) => Check {
            name: "toolchain".to_owned(),
            ok: true,
            detail: format!(
                "seal {} (embedded compiler, no project pin)",
                seal_toolchain::EMBEDDED_VERSION
            ),
        },
    };
    let spec = match project.target_spec(target) {
        Ok(spec) => spec,
        Err(error) => {
            return (
                toolchain,
                Check {
                    name: "target".to_owned(),
                    ok: false,
                    detail: error.message().to_owned(),
                },
            );
        }
    };
    let mut problems = Vec::new();
    if let Some(linker) = spec.linker() {
        let found = if linker.contains(['/', '\\']) {
            Path::new(linker).is_file()
        } else {
            find_on_path(linker).is_some()
        };
        if !found {
            problems.push(format!("linker `{linker}` is not available"));
        }
    }
    if let Some(sysroot) = spec.sysroot()
        && !sysroot.is_dir()
    {
        problems.push(format!("sysroot `{}` is missing", sysroot.display()));
    }
    if let Some(script) = spec.linker_script()
        && !script.is_file()
    {
        problems.push(format!("linker script `{}` is missing", script.display()));
    }
    let target_check = Check {
        name: "target".to_owned(),
        ok: problems.is_empty(),
        detail: if problems.is_empty() {
            format!(
                "{} (runtime={}, panic={})",
                spec.triple(),
                spec.runtime(),
                spec.panic()
            )
        } else {
            problems.join("; ")
        },
    };
    (toolchain, target_check)
}

fn find_on_path(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
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

fn check_sandbox() -> Check {
    let sandbox = Sandbox::detect();
    if sandbox.namespaces_available() {
        Check {
            name: "sandbox".to_owned(),
            ok: true,
            detail: format!(
                "namespaces (bwrap: {})",
                sandbox
                    .bwrap()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
        }
    } else {
        Check {
            name: "sandbox".to_owned(),
            ok: true,
            detail: "portable (bubblewrap is unavailable; filesystem isolation is off)".to_owned(),
        }
    }
}

fn check_registry(project: &Project) -> Check {
    let Some(registry) = &project.registry else {
        return Check {
            name: "registry".to_owned(),
            ok: true,
            detail: "no registry configured (path dependencies only)".to_owned(),
        };
    };
    let Some(url) = &registry.url else {
        return Check {
            name: "registry".to_owned(),
            ok: false,
            detail: "the `[registry]` table has no `url`".to_owned(),
        };
    };
    if !url.starts_with("https://") && !(registry.allow_insecure && url.starts_with("http://")) {
        return Check {
            name: "registry".to_owned(),
            ok: false,
            detail: format!("the registry URL `{url}` needs https or `allow-insecure`"),
        };
    }
    let token = registry
        .token_env
        .as_ref()
        .map(|name| {
            if std::env::var_os(name).is_some() {
                format!("token from `{name}`")
            } else {
                format!("`{name}` is not set")
            }
        })
        .unwrap_or_else(|| "no token variable".to_owned());
    Check {
        name: "registry".to_owned(),
        ok: true,
        detail: format!(
            "{url} ({token}, {}offline)",
            if registry.offline { "" } else { "not " }
        ),
    }
}

fn check_manifest(project: &Project) -> Check {
    let path = project.root.join("seal.toml");
    if !path.is_file() {
        return Check {
            name: "manifest".to_owned(),
            ok: false,
            detail: format!("`{}` is missing", path.display()),
        };
    }
    Check {
        name: "manifest".to_owned(),
        ok: true,
        detail: format!(
            "{} package(s), {} warning(s)",
            project.packages.len(),
            project.warnings.len()
        ),
    }
}

fn check_disk(project: &Project, fix: bool) -> Check {
    let root = project.root.join("seal");
    if fix {
        let _ = std::fs::create_dir_all(root.join("cache"));
        let _ = std::fs::create_dir_all(root.join("target"));
    }
    let used = directory_size(&root);
    Check {
        name: "disk".to_owned(),
        ok: true,
        detail: format!("{:.1} MiB under `seal/`", used as f64 / (1024.0 * 1024.0)),
    }
}

fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            total += directory_size(&entry.path());
        } else if let Ok(metadata) = entry.metadata() {
            total += metadata.len();
        }
    }
    total
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
