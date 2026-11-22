//! The sandboxed action worker.
//!
//! Inputs are materialized under a fresh private work directory, the command
//! runs with an empty environment plus only the declared variables, and every
//! output is read back from the work directory. Names are validated before any
//! path is joined, so a malicious Directory cannot escape the work tree.

use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use prost::Message;
use seal_cas::Digest;
use seal_remote::proto;
use seal_sandbox::{SandboxPolicy, SandboxRequest};

use crate::cas::wire_digest;
use crate::error::{ServerError, ServerResult};
use crate::state::State;

const MAX_DIRECTORY_DEPTH: usize = 64;
const INLINE_LOG_BYTES: usize = 1024 * 1024;

/// Runs one action and returns its result without caching it.
pub(crate) fn run(
    state: &State,
    action: &proto::Action,
    command: &proto::Command,
) -> ServerResult<proto::ActionResult> {
    let timeout = action_timeout(state, action)?;
    let input_root = action
        .input_root_digest
        .as_ref()
        .ok_or_else(|| ServerError::invalid("the Action has no input root"))?;
    let (root_digest, root_size) = parse_wire(input_root)?;
    let work = state.store.work_directory()?;
    let guard = WorkGuard { path: work.clone() };
    let mut input_bytes = 0_u64;
    materialize_directory(state, root_digest, root_size, &work, &mut input_bytes, 0)?;
    if input_bytes > state.config.max_input_bytes {
        return Err(ServerError::invalid(format!(
            "the input tree exceeds the {} byte limit",
            state.config.max_input_bytes
        )));
    }
    let working_directory = if command.working_directory.is_empty() {
        work.clone()
    } else {
        safe_join(&work, &command.working_directory)?
    };
    if !working_directory.is_dir() {
        return Err(ServerError::invalid(format!(
            "the working directory `{}` is not a directory in the input root",
            command.working_directory
        )));
    }
    let program = resolve_program(
        command
            .arguments
            .first()
            .ok_or_else(|| ServerError::invalid("the Command has no arguments"))?,
        &working_directory,
        state.config.tool_path.as_deref(),
    )?;
    let arguments: Vec<OsString> = command
        .arguments
        .iter()
        .skip(1)
        .map(OsString::from)
        .collect();
    for output in &command.output_paths {
        seal_remote::validate_relative_path(output)
            .map_err(|error| ServerError::invalid(error.to_string()))?;
        let path = safe_join(&working_directory, output)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ServerError::internal(format!(
                    "cannot create the output directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
    }
    let mut policy = SandboxPolicy::new()
        .read(&work)
        .write(&work)
        .read(
            program
                .parent()
                .ok_or_else(|| ServerError::invalid("the program has no parent directory"))?,
        )
        .working_dir(&working_directory)
        .timeout(timeout)
        .allow_portable(state.config.allow_portable_sandbox);
    if let Some(path) = std::env::var_os("PATH")
        && !command
            .environment_variables
            .iter()
            .any(|variable| variable.name == "PATH")
    {
        policy = policy.env("PATH", path.to_string_lossy().into_owned());
    }
    for variable in &command.environment_variables {
        if variable.name.is_empty() || variable.name.contains(['=', '\0']) {
            return Err(ServerError::invalid(format!(
                "invalid environment variable name `{}`",
                variable.name
            )));
        }
        policy = policy.env(variable.name.clone(), variable.value.clone());
    }
    let request = SandboxRequest::new(program.as_os_str())
        .args(arguments)
        .policy(policy);
    let output = state.sandbox.run(&request).map_err(|error| {
        if error.code() == "E8044" {
            ServerError::timeout(error.to_string())
        } else {
            ServerError::sandbox(error.to_string())
        }
    })?;
    let exit_code = output
        .status
        .code()
        .unwrap_or_else(|| 128 + signal_number(&output.status));
    let mut result = proto::ActionResult {
        output_files: Vec::new(),
        output_directories: Vec::new(),
        output_symlinks: Vec::new(),
        exit_code,
        stdout_raw: Vec::new(),
        stdout_digest: None,
        stderr_raw: Vec::new(),
        stderr_digest: None,
    };
    for name in &command.output_paths {
        let path = safe_join(&working_directory, name)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                let bytes = fs::read(&path).map_err(|error| {
                    ServerError::internal(format!(
                        "cannot read the output `{}`: {error}",
                        path.display()
                    ))
                })?;
                let digest = Digest::of_bytes(&bytes);
                state.store.put_blob(digest, bytes.len() as u64, &bytes)?;
                result.output_files.push(proto::OutputFile {
                    path: name.clone(),
                    digest: Some(wire_digest(digest, bytes.len() as u64)),
                    is_executable: is_executable(&metadata),
                    contents: Vec::new(),
                });
            }
            Ok(metadata) if metadata.is_dir() => {
                return Err(ServerError::unsupported(format!(
                    "the output `{name}` is a directory; only file outputs are served"
                )));
            }
            Ok(_) => {
                return Err(ServerError::unsupported(format!(
                    "the output `{name}` is a symlink or special file"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ServerError::internal(format!(
                    "cannot inspect the output `{name}`: {error}"
                )));
            }
        }
    }
    publish_log(state, &mut result, true, output.stdout)?;
    publish_log(state, &mut result, false, output.stderr)?;
    drop(guard);
    Ok(result)
}

fn action_timeout(state: &State, action: &proto::Action) -> ServerResult<Duration> {
    let Some(timeout) = &action.timeout else {
        return Ok(state.config.max_action_timeout);
    };
    if timeout.seconds < 0 || !(0..1_000_000_000).contains(&timeout.nanos) {
        return Err(ServerError::invalid("the action timeout is invalid"));
    }
    let duration = Duration::new(timeout.seconds as u64, timeout.nanos as u32);
    if duration.is_zero() || duration > state.config.max_action_timeout {
        return Err(ServerError::invalid(format!(
            "the action timeout must be between 1 ns and {} seconds",
            state.config.max_action_timeout.as_secs()
        )));
    }
    Ok(duration)
}

fn materialize_directory(
    state: &State,
    digest: Digest,
    size: u64,
    destination: &Path,
    total: &mut u64,
    depth: usize,
) -> ServerResult<()> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(ServerError::invalid(
            "the input directory tree exceeds the depth limit",
        ));
    }
    let bytes = state.store.get_blob(digest)?;
    if bytes.len() as u64 != size {
        return Err(ServerError::integrity(
            "an input Directory has the wrong size",
        ));
    }
    let directory = proto::Directory::decode(bytes.as_slice())
        .map_err(|error| ServerError::invalid(format!("invalid Directory: {error}")))?;
    fs::create_dir_all(destination).map_err(|error| {
        ServerError::internal(format!(
            "cannot create `{}`: {error}",
            destination.display()
        ))
    })?;
    for file in &directory.files {
        validate_node_name(&file.name)?;
        let wire = file
            .digest
            .as_ref()
            .ok_or_else(|| ServerError::invalid(format!("`{}` has no digest", file.name)))?;
        let (file_digest, file_size) = parse_wire(wire)?;
        *total = total.saturating_add(file_size);
        if *total > state.config.max_input_bytes {
            return Err(ServerError::invalid(format!(
                "the input tree exceeds the {} byte limit",
                state.config.max_input_bytes
            )));
        }
        let bytes = state.store.get_blob(file_digest)?;
        if bytes.len() as u64 != file_size {
            return Err(ServerError::integrity(format!(
                "the input `{}` has the wrong size",
                file.name
            )));
        }
        let path = destination.join(&file.name);
        fs::write(&path, &bytes).map_err(|error| {
            ServerError::internal(format!("cannot write `{}`: {error}", path.display()))
        })?;
        if file.is_executable {
            make_executable(&path)?;
        }
    }
    for child in &directory.directories {
        validate_node_name(&child.name)?;
        let wire = child
            .digest
            .as_ref()
            .ok_or_else(|| ServerError::invalid(format!("`{}` has no digest", child.name)))?;
        let (child_digest, child_size) = parse_wire(wire)?;
        materialize_directory(
            state,
            child_digest,
            child_size,
            &destination.join(&child.name),
            total,
            depth + 1,
        )?;
    }
    if !directory.symlinks.is_empty() {
        return Err(ServerError::unsupported(
            "symlink inputs are not supported by this endpoint",
        ));
    }
    Ok(())
}

fn parse_wire(wire: &proto::Digest) -> ServerResult<(Digest, u64)> {
    if wire.size_bytes < 0 {
        return Err(ServerError::invalid("a digest has a negative size"));
    }
    Ok((
        Digest::parse(&wire.hash).map_err(|error| ServerError::integrity(error.to_string()))?,
        wire.size_bytes as u64,
    ))
}

fn validate_node_name(name: &str) -> ServerResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(ServerError::invalid(format!(
            "the input node name `{name}` is invalid"
        )));
    }
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> ServerResult<PathBuf> {
    seal_remote::validate_relative_path(relative)
        .map_err(|error| ServerError::invalid(error.to_string()))?;
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(segment) => path.push(segment),
            _ => {
                return Err(ServerError::invalid(format!(
                    "the path `{relative}` escapes the working directory"
                )));
            }
        }
    }
    Ok(path)
}

fn resolve_program(
    program: &str,
    working_directory: &Path,
    tool_path: Option<&Path>,
) -> ServerResult<PathBuf> {
    if program.trim().is_empty() || program.contains('\0') {
        return Err(ServerError::invalid("the command program is empty"));
    }
    let candidate = PathBuf::from(program);
    if candidate.is_absolute() {
        let allowed = tool_path.is_some_and(|directory| candidate.starts_with(directory));
        if allowed && candidate.is_file() {
            return Ok(candidate);
        }
        return Err(ServerError::invalid(format!(
            "the absolute program `{program}` is outside the worker tool path"
        )));
    }
    if program.contains('/') {
        let joined = safe_join(working_directory, program)?;
        return joined.is_file().then_some(joined).ok_or_else(|| {
            ServerError::invalid(format!("the program `{program}` does not exist"))
        });
    }
    if let Some(directory) = tool_path {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(ServerError::invalid(format!(
        "the program `{program}` was not found on the worker"
    )))
}

fn publish_log(
    state: &State,
    result: &mut proto::ActionResult,
    stdout: bool,
    bytes: Vec<u8>,
) -> ServerResult<()> {
    if bytes.len() <= INLINE_LOG_BYTES {
        if stdout {
            result.stdout_raw = bytes;
        } else {
            result.stderr_raw = bytes;
        }
        return Ok(());
    }
    let digest = Digest::of_bytes(&bytes);
    state.store.put_blob(digest, bytes.len() as u64, &bytes)?;
    if stdout {
        result.stdout_digest = Some(wire_digest(digest, bytes.len() as u64));
    } else {
        result.stderr_digest = Some(wire_digest(digest, bytes.len() as u64));
    }
    Ok(())
}

fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn make_executable(path: &Path) -> ServerResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|error| {
            ServerError::internal(format!(
                "cannot make `{}` executable: {error}",
                path.display()
            ))
        })?;
    }
    let _ = path;
    Ok(())
}

fn signal_number(status: &std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        0
    }
}

struct WorkGuard {
    path: PathBuf,
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::resolve_program;

    #[test]
    fn a_program_cannot_escape_the_work_tree_or_tool_path() {
        let work = Path::new("/tmp/seal-worker-test-work");
        assert!(resolve_program("../bin/sh", work, None).is_err());
        assert!(resolve_program("/bin/sh", work, None).is_err());
        assert!(resolve_program("/bin/sh", work, Some(Path::new("/opt/seal"))).is_err());
    }
}
