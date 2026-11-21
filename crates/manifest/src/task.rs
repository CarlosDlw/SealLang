//! Declarative project tasks.
//!
//! A task is a list of steps. Each step names a program and an argument vector;
//! no shell is involved, so quoting, globbing, and pipelines are explicit
//! decisions made by the task author. The sandbox clears the environment and
//! exposes only the values declared by the step.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// One `[tasks.<name>]` table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TaskManifest {
    /// A short human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Steps run in order.
    #[serde(default)]
    pub steps: Vec<TaskStepManifest>,
}

/// One task step.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TaskStepManifest {
    /// The program to execute. It is not interpreted by a shell.
    pub program: String,
    /// Arguments passed unchanged.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment values visible to the step.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Working directory relative to the project root.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Wall-clock limit in seconds.
    #[serde(default)]
    pub timeout: Option<u64>,
}
