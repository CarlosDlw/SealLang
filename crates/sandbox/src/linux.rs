//! Linux namespace isolation through bubblewrap.
//!
//! The backend builds an empty mount namespace, exposes only the host toolchain
//! directories needed to load a dynamically linked program, then mounts declared
//! inputs read-only and output directories read-write. Networking is removed by
//! `--unshare-all` unless the policy allows it. `--die-with-parent` makes sure a
//! killed supervisor takes the command with it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::SandboxError;
use crate::policy::SandboxPolicy;
use crate::process;

/// Host directories that a dynamically linked program needs.
const RUNTIME_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/localtime",
];

/// Returns the absolute path of `bwrap`, when it is executable on `PATH`.
pub(crate) fn available() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in ["bwrap", "bubblewrap"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Runs `program` with `arguments` under namespace isolation.
pub(crate) fn run(
    bwrap: &Path,
    program: &OsStr,
    arguments: &[OsString],
    policy: &SandboxPolicy,
) -> Result<process::Captured, SandboxError> {
    let mut command = Command::new(bwrap);
    command.args(build_arguments(program, arguments, policy)?);
    process::prepare(&mut command);
    for (key, value) in policy.environment() {
        command.env(key, value);
    }
    process::capture(bwrap.as_os_str(), command, policy.wall_clock_timeout())
}

/// Builds the bubblewrap argument vector for one policy.
pub(crate) fn build_arguments(
    program: &OsStr,
    arguments: &[OsString],
    policy: &SandboxPolicy,
) -> Result<Vec<OsString>, SandboxError> {
    let mut result: Vec<OsString> = Vec::new();
    result.push("--unshare-all".into());
    if policy.network_allowed() {
        result.push("--share-net".into());
    }
    result.push("--die-with-parent".into());
    result.push("--new-session".into());
    result.push("--tmpfs".into());
    result.push("/".into());
    result.push("--proc".into());
    result.push("/proc".into());
    result.push("--dev".into());
    result.push("/dev".into());
    result.push("--tmpfs".into());
    result.push("/tmp".into());

    for path in RUNTIME_PATHS {
        let path = Path::new(path);
        if path.exists() {
            bind(&mut result, "--ro-bind", path)?;
        }
    }

    for path in policy.read_paths() {
        let path = resolve(path)?;
        bind(&mut result, "--ro-bind", &path)?;
    }
    for path in policy.write_paths() {
        if !path.exists() {
            std::fs::create_dir_all(path).map_err(|error| SandboxError::Policy {
                path: path.display().to_string(),
                reason: format!("cannot create the output directory: {error}"),
            })?;
        }
        let path = resolve(path)?;
        bind(&mut result, "--bind", &path)?;
    }

    if let Some(directory) = policy.working_directory() {
        let directory = resolve(directory)?;
        result.push("--chdir".into());
        result.push(directory.into_os_string());
    }

    result.push("--".into());
    result.push(program.to_owned());
    result.extend(arguments.iter().cloned());
    Ok(result)
}

/// Resolves a policy path and rejects it when it does not exist.
fn resolve(path: &Path) -> Result<PathBuf, SandboxError> {
    path.canonicalize().map_err(|error| SandboxError::Policy {
        path: path.display().to_string(),
        reason: format!("cannot resolve: {error}"),
    })
}

/// Adds a bind mount and creates every missing ancestor destination.
fn bind(result: &mut Vec<OsString>, mode: &str, source: &Path) -> Result<(), SandboxError> {
    for ancestor in ancestors(source) {
        result.push("--dir".into());
        result.push(ancestor.into_os_string());
    }
    result.push(mode.into());
    result.push(source.as_os_str().to_owned());
    result.push(source.as_os_str().to_owned());
    Ok(())
}

/// Returns the destination ancestors of `path`, outermost first.
fn ancestors(path: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory.as_os_str().is_empty() || directory == Path::new("/") {
            break;
        }
        directories.push(directory.to_path_buf());
        current = directory.parent();
    }
    directories.reverse();
    directories
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{ancestors, build_arguments};
    use crate::policy::SandboxPolicy;

    #[test]
    fn network_is_unshared_by_default_and_shared_only_on_request() {
        let default = build_arguments(
            Path::new("/bin/echo").as_os_str(),
            &[OsString::from("hi")],
            &SandboxPolicy::new(),
        )
        .expect("the policy builds");
        assert!(default.contains(&OsString::from("--unshare-all")));
        assert!(!default.contains(&OsString::from("--share-net")));

        let shared = build_arguments(
            Path::new("/bin/echo").as_os_str(),
            &[],
            &SandboxPolicy::new().allow_network(true),
        )
        .expect("the policy builds");
        assert!(shared.contains(&OsString::from("--share-net")));
    }

    #[test]
    fn ancestors_are_outermost_first() {
        assert_eq!(
            ancestors(Path::new("/home/user/project/src/main.sl")),
            vec![
                Path::new("/home").to_path_buf(),
                Path::new("/home/user").to_path_buf(),
                Path::new("/home/user/project").to_path_buf(),
                Path::new("/home/user/project/src").to_path_buf(),
            ]
        );
    }
}
