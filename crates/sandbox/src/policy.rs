//! Sandbox policy: what a command may read, write, and see.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// The filesystem, environment, and resource limits of one sandboxed command.
///
/// Paths are absolute or resolved against the process working directory before
/// the sandbox starts. Inputs are mounted read-only, output directories are
/// mounted read-write, and nothing else from the host is visible when namespace
/// isolation is available.
#[derive(Clone, Debug, Default)]
pub struct SandboxPolicy {
    pub(crate) read_paths: Vec<PathBuf>,
    pub(crate) write_paths: Vec<PathBuf>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) working_dir: Option<PathBuf>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) allow_network: bool,
    pub(crate) allow_portable: bool,
    pub(crate) require_isolation: bool,
}

impl SandboxPolicy {
    /// Creates a policy with no visible paths, no environment, and no network.
    pub fn new() -> Self {
        Self::default()
    }

    /// Exposes `path` read-only inside the sandbox.
    pub fn read(mut self, path: impl Into<PathBuf>) -> Self {
        self.read_paths.push(path.into());
        self
    }

    /// Exposes `path` read-write inside the sandbox.
    pub fn write(mut self, path: impl Into<PathBuf>) -> Self {
        self.write_paths.push(path.into());
        self
    }

    /// Sets one environment variable.
    ///
    /// The environment starts empty: a command only sees the values added here,
    /// so `HOME`, tokens, and locale settings never leak from the host.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Sets the process working directory.
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Sets the wall-clock timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Allows network access. Network is off by default.
    pub fn allow_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// Allows the portable backend when namespaces are unavailable.
    pub fn allow_portable(mut self, allow: bool) -> Self {
        self.allow_portable = allow;
        self
    }

    /// Requires namespace isolation and rejects the portable fallback.
    pub fn require_isolation(mut self, require: bool) -> Self {
        self.require_isolation = require;
        self
    }

    /// Returns the read-only paths.
    pub fn read_paths(&self) -> &[PathBuf] {
        &self.read_paths
    }

    /// Returns the read-write paths.
    pub fn write_paths(&self) -> &[PathBuf] {
        &self.write_paths
    }

    /// Returns the environment visible to the command.
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Returns the working directory.
    pub fn working_directory(&self) -> Option<&PathBuf> {
        self.working_dir.as_ref()
    }

    /// Returns the timeout.
    pub fn wall_clock_timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns whether network access is allowed.
    pub fn network_allowed(&self) -> bool {
        self.allow_network
    }

    /// Returns whether the portable fallback is allowed.
    pub fn portable_allowed(&self) -> bool {
        self.allow_portable
    }

    /// Returns whether namespace isolation is required.
    pub fn isolation_required(&self) -> bool {
        self.require_isolation
    }
}
