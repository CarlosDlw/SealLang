//! Options shared by every command that analyzes source files.

use std::path::PathBuf;

use clap::Args;

use super::diagnostics::DiagnosticArgs;
use super::target::TargetArgs;

/// The source files and the environment they are analyzed under.
#[derive(Debug, Args)]
pub(crate) struct CompileArgs {
    /// Source files to analyze.
    #[arg(value_name = "INPUT")]
    pub(crate) inputs: Vec<PathBuf>,

    /// Compile every input as one package instead of one program per file.
    ///
    /// Module paths come from each file's `module` declaration; a file without
    /// one contributes to the package root. Without this flag every input is an
    /// independent single-file program.
    #[arg(long, help_heading = "Compilation")]
    pub(crate) plan: bool,

    /// Read the package graph from a versioned JSON compile plan.
    ///
    /// `seal` writes this file; it replaces positional inputs and `--plan`.
    #[arg(long = "plan-file", value_name = "PATH", help_heading = "Compilation")]
    pub(crate) plan_file: Option<PathBuf>,

    /// Expose a module path in the package API, for example `a::b` or `root`.
    ///
    /// May be repeated. Only meaningful with `--plan`; when omitted the package
    /// root is exposed, so a binary package can still use `public` items.
    #[arg(long = "expose", value_name = "PATH", help_heading = "Compilation")]
    pub(crate) expose: Vec<String>,

    /// Ignore every package prelude named by the compile plan.
    ///
    /// This is the low-level opt-out; `seal` exposes the same switch through
    /// `--no-prelude`.
    #[arg(long = "no-prelude", help_heading = "Compilation")]
    pub(crate) no_prelude: bool,

    #[command(flatten)]
    pub(crate) target: TargetArgs,

    #[command(flatten)]
    pub(crate) diagnostics: DiagnosticArgs,
}
