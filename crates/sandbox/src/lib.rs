//! Hermetic execution for build tools and generators.
//!
//! On Linux the sandbox uses bubblewrap: an empty mount namespace exposes only
//! the host toolchain directories and the paths named by a [`SandboxPolicy`],
//! networking is removed, and the command runs with an empty environment. Other
//! hosts fall back to [`Isolation::Portable`] when the policy allows it, which
//! preserves the timeout and environment contract but cannot hide host files.
//!
//! Every command runs with an empty environment by default. Callers add exactly
//! the variables a tool needs, so `HOME`, tokens, and locale settings never leak
//! into generators, native probes, or tasks.

#![warn(missing_docs)]

mod error;
mod linux;
mod policy;
mod portable;
mod process;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

pub use error::SandboxError;
pub use policy::SandboxPolicy;

/// The isolation a command actually received.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// A mount, PID, and network namespace through bubblewrap.
    Namespaces,
    /// No filesystem or network isolation; only environment and timeout.
    Portable,
}

impl Isolation {
    /// Returns the stable spelling used in logs and metadata.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Namespaces => "namespaces",
            Self::Portable => "portable",
        }
    }
}

/// One command to run inside a sandbox.
#[derive(Clone, Debug)]
pub struct SandboxRequest {
    program: OsString,
    arguments: Vec<OsString>,
    policy: SandboxPolicy,
}

impl SandboxRequest {
    /// Creates a request for `program` with a policy that allows no host access.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            policy: SandboxPolicy::new(),
        }
    }

    /// Adds one argument.
    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    /// Adds several arguments in order.
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Sets the policy.
    pub fn policy(mut self, policy: SandboxPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Returns the program.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Returns the arguments.
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns the policy.
    pub fn sandbox_policy(&self) -> &SandboxPolicy {
        &self.policy
    }
}

/// The captured result of a sandboxed command.
#[derive(Clone, Debug)]
pub struct SandboxOutput {
    /// The exit status.
    pub status: ExitStatus,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
    /// The isolation that was actually applied.
    pub isolation: Isolation,
}

impl SandboxOutput {
    /// Returns whether the command exited successfully.
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// Returns standard output as lossy UTF-8.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Returns standard error as lossy UTF-8.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A sandbox backend selected for this host.
#[derive(Clone, Debug)]
pub struct Sandbox {
    bwrap: Option<PathBuf>,
}

impl Sandbox {
    /// Selects the best backend available on this host.
    pub fn detect() -> Self {
        Self {
            bwrap: linux::available(),
        }
    }

    /// Returns whether namespace isolation is available.
    pub fn namespaces_available(&self) -> bool {
        self.bwrap.is_some()
    }

    /// Returns the bubblewrap path, when namespace isolation is available.
    pub fn bwrap(&self) -> Option<&Path> {
        self.bwrap.as_deref()
    }

    /// Runs one request and returns its captured output.
    pub fn run(&self, request: &SandboxRequest) -> Result<SandboxOutput, SandboxError> {
        let (captured, isolation) = match &self.bwrap {
            Some(bwrap) => (
                linux::run(bwrap, &request.program, &request.arguments, &request.policy)?,
                Isolation::Namespaces,
            ),
            None => {
                if request.policy.isolation_required() {
                    return Err(SandboxError::Unsupported {
                        reason: "bubblewrap is not installed and the policy requires namespace isolation"
                            .to_owned(),
                    });
                }
                if !request.policy.portable_allowed() {
                    return Err(SandboxError::Unsupported {
                        reason: "bubblewrap is not installed; install bubblewrap or allow the portable fallback"
                            .to_owned(),
                    });
                }
                (
                    portable::run(&request.program, &request.arguments, &request.policy)?,
                    Isolation::Portable,
                )
            }
        };
        Ok(SandboxOutput {
            status: captured.status,
            stdout: captured.stdout,
            stderr: captured.stderr,
            isolation,
        })
    }

    /// Runs one request and fails unless the command exits successfully.
    pub fn run_checked(&self, request: &SandboxRequest) -> Result<SandboxOutput, SandboxError> {
        let output = self.run(request)?;
        process::check_status(&request.program, output.status, &output.stderr)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use super::{Isolation, Sandbox, SandboxError, SandboxPolicy, SandboxRequest};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "{prefix}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn shell(script: &str) -> SandboxRequest {
        SandboxRequest::new("/bin/sh").args(["-c", script])
    }

    #[test]
    fn the_environment_starts_empty_and_only_policy_values_are_visible() {
        let sandbox = Sandbox::detect();
        let request = shell("printf '%s|%s' \"$HOME\" \"$SEAL_TEST\"")
            .policy(SandboxPolicy::new().env("SEAL_TEST", "visible"));
        let output = sandbox.run(&request).expect("the command runs");
        assert!(output.success());
        assert_eq!(output.stdout_text(), "|visible");
    }

    #[test]
    fn a_timeout_kills_the_process_with_e8044() {
        let sandbox = Sandbox::detect();
        let request = shell("sleep 5").policy(
            SandboxPolicy::new()
                .timeout(Duration::from_millis(100))
                .allow_portable(true),
        );
        let error = sandbox.run(&request).expect_err("the command times out");
        assert_eq!(error.code(), "E8044");
        assert!(matches!(error, SandboxError::Timeout { .. }));
    }

    #[test]
    fn an_unsuccessful_command_reports_its_stderr() {
        let sandbox = Sandbox::detect();
        let request =
            shell("echo denied >&2; exit 7").policy(SandboxPolicy::new().allow_portable(true));
        let error = sandbox
            .run_checked(&request)
            .expect_err("the command fails");
        assert_eq!(error.code(), "E8041");
        assert!(error.to_string().contains("denied"));
    }

    #[test]
    fn namespace_isolation_hides_files_outside_the_policy() {
        let sandbox = Sandbox::detect();
        if !sandbox.namespaces_available() {
            return;
        }
        let directory = TempDir::new("seal-sandbox-read");
        let allowed = directory.path().join("allowed");
        std::fs::create_dir_all(&allowed).expect("allowed directory");
        let visible = allowed.join("visible.txt");
        std::fs::write(&visible, b"visible").expect("visible file");
        let secret = directory.path().join("secret.txt");
        std::fs::write(&secret, b"secret").expect("secret file");

        let policy = SandboxPolicy::new()
            .read(&allowed)
            .working_dir(&allowed)
            .allow_portable(true);
        let visible_output = sandbox
            .run(&shell("cat visible.txt").policy(policy.clone()))
            .expect("the visible file is readable");
        assert!(visible_output.success());
        assert_eq!(visible_output.stdout_text(), "visible");
        assert_eq!(visible_output.isolation, Isolation::Namespaces);

        let hidden = sandbox
            .run(&shell(&format!("cat {}", secret.display())).policy(policy))
            .expect("the command runs");
        assert!(
            !hidden.success(),
            "a file outside the policy must be hidden"
        );
    }

    #[test]
    fn namespace_isolation_exposes_output_directories_as_writable() {
        let sandbox = Sandbox::detect();
        if !sandbox.namespaces_available() {
            return;
        }
        let directory = TempDir::new("seal-sandbox-write");
        let output = directory.path().join("out");
        let policy = SandboxPolicy::new()
            .write(&output)
            .working_dir(&output)
            .allow_portable(true);
        let result = sandbox
            .run(&shell("printf generated > artifact.txt").policy(policy))
            .expect("the command runs");
        assert!(result.success(), "{}", result.stderr_text());
        assert_eq!(
            std::fs::read(output.join("artifact.txt")).expect("the output exists"),
            b"generated"
        );
    }

    #[test]
    fn network_is_unreachable_with_namespace_isolation() {
        let sandbox = Sandbox::detect();
        if !sandbox.namespaces_available() || !Path::new("/bin/bash").is_file() {
            return;
        }
        let request = SandboxRequest::new("/bin/bash")
            .args(["-c", "exec 3<>/dev/tcp/1.1.1.1/80"])
            .policy(
                SandboxPolicy::new()
                    .timeout(Duration::from_secs(3))
                    .allow_portable(true),
            );
        let output = sandbox.run(&request).expect("the command runs");
        assert!(
            !output.success(),
            "a sandboxed command must not reach the network"
        );
    }

    #[test]
    fn requiring_namespaces_on_a_portable_host_fails_clearly() {
        let sandbox = Sandbox { bwrap: None };
        let request = shell("true").policy(SandboxPolicy::new().require_isolation(true));
        let error = sandbox.run(&request).expect_err("isolation is unavailable");
        assert_eq!(error.code(), "E8041");
    }
}
