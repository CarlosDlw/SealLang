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

fn find_linker() -> Option<String> {
    ["cc", "clang", "gcc"].into_iter().find_map(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|_| candidate.to_owned())
    })
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
    let expected = project.resolve_lockfile(target);
    let current = match seal_project::Lockfile::load(&path) {
        Ok(lockfile) => lockfile == expected,
        Err(_) => false,
    };
    if current {
        return Check {
            name: "lockfile".to_owned(),
            ok: true,
            detail: path.display().to_string(),
        };
    }
    if fix {
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
