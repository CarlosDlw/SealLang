//! Argument parsing and command dispatch.

pub(crate) mod build;
mod check;
mod doctor;
mod init;
pub(crate) mod metadata;
pub(crate) mod new;
mod run;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::ui;

/// Template selected by `seal new` and `seal init`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Template {
    /// A package with an executable.
    Bin,
    /// A package with a library.
    Lib,
    /// A root project with two members.
    Workspace,
}

/// Version-control setup for a new project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Vcs {
    /// Initialize a Git repository.
    Git,
    /// Do not create version-control files.
    None,
}

/// Color policy for compiler diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ColorChoice {
    /// Use color only on a terminal.
    Auto,
    /// Always use color.
    Always,
    /// Never use color.
    Never,
}

/// Diagnostic output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum MessageFormat {
    /// Human-readable output.
    Human,
    /// One JSON object per event.
    Json,
}

/// The project CLI.
#[derive(Debug, Parser)]
#[command(
    name = "seal",
    version,
    about = "The Seal project tool",
    long_about = "Seal builds, checks, and runs multi-file projects.\n\n\
                  Run `seal help <COMMAND>` for the options of one command.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// Directory of the project; defaults to the current directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) project: Option<PathBuf>,

    /// Suppress status output.
    #[arg(long, global = true)]
    pub(crate) quiet: bool,

    /// Print the commands and paths the tool uses.
    #[arg(long, global = true)]
    pub(crate) verbose: bool,

    /// Diagnostic output format.
    #[arg(long, global = true, value_enum, default_value = "human")]
    pub(crate) message_format: MessageFormat,

    /// Color policy for compiler diagnostics.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub(crate) color: ColorChoice,

    /// The action to perform.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The available commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Create a new project.
    New(NewArgs),
    /// Add Seal files to an existing directory.
    Init(InitArgs),
    /// Build targets.
    Build(BuildArgs),
    /// Build and run one executable.
    Run(RunArgs),
    /// Analyze targets without writing artifacts.
    Check(CheckArgs),
    /// Print the project model as JSON.
    Metadata(MetadataArgs),
    /// Check the toolchain, linker, cache, and lockfile.
    Doctor(DoctorArgs),
}

/// Options shared by build-like commands.
#[derive(Debug, Args)]
pub(crate) struct Selection {
    /// Targets to select by name.
    #[arg(value_name = "TARGET")]
    pub(crate) targets: Vec<String>,

    /// Select one package by name.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,

    /// Select every package in the project.
    #[arg(long)]
    pub(crate) workspace: bool,

    /// Select all targets, including tests, benches, and examples.
    #[arg(long)]
    pub(crate) all_targets: bool,

    /// Select a named profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,

    /// Use the release profile.
    #[arg(long, conflicts_with = "profile")]
    pub(crate) release: bool,

    /// Compile for a target triple.
    #[arg(long, value_name = "TRIPLE")]
    pub(crate) target_triple: Option<String>,

    /// Enable features; features are recorded in the plan.
    #[arg(long, value_delimiter = ',', value_name = "FEATURE")]
    pub(crate) features: Vec<String>,

    /// Enable every feature.
    #[arg(long, conflicts_with = "features")]
    pub(crate) all_features: bool,

    /// Disable default features.
    #[arg(long)]
    pub(crate) no_default_features: bool,

    /// Fail if the lockfile would change.
    #[arg(long)]
    pub(crate) locked: bool,

    /// Fail if the lockfile would change and forbid network access.
    #[arg(long)]
    pub(crate) frozen: bool,
}

/// `seal new`.
#[derive(Debug, Args)]
pub(crate) struct NewArgs {
    /// Directory to create.
    #[arg(value_name = "PATH", default_value = ".")]
    pub(crate) path: PathBuf,

    /// Project template.
    #[arg(long, value_enum, default_value = "bin")]
    pub(crate) template: Template,

    /// Version-control setup.
    #[arg(long, value_enum, default_value = "git")]
    pub(crate) vcs: Vcs,

    /// Do not ask for confirmation.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `seal init`.
#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    /// Project template.
    #[arg(long, value_enum, default_value = "bin")]
    pub(crate) template: Template,

    /// Version-control setup.
    #[arg(long, value_enum, default_value = "git")]
    pub(crate) vcs: Vcs,

    /// Do not ask for confirmation.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `seal build`.
#[derive(Debug, Args)]
pub(crate) struct BuildArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// Pass an argument through to the linker.
    #[arg(long = "link-arg", value_name = "ARG")]
    pub(crate) link_args: Vec<String>,
}

/// `seal run`.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// Select a binary by name.
    #[arg(long, value_name = "NAME")]
    pub(crate) bin: Option<String>,

    /// Arguments forwarded to the program.
    #[arg(last = true, value_name = "ARGS")]
    pub(crate) program_args: Vec<OsString>,
}

/// `seal check`.
#[derive(Debug, Args)]
pub(crate) struct CheckArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,
}

/// `seal metadata`.
#[derive(Debug, Args)]
pub(crate) struct MetadataArgs {
    /// Omit dependencies from package output.
    #[arg(long)]
    pub(crate) no_deps: bool,

    /// Print JSON (the default and only format).
    #[arg(long)]
    pub(crate) json: bool,

    /// Output format; only JSON is available.
    #[arg(long, value_enum, default_value = "json")]
    pub(crate) format: MetadataFormat,
}

/// Metadata output formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum MetadataFormat {
    /// Versioned JSON.
    Json,
}

/// `seal doctor`.
#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,

    /// Repair the lockfile and cache directory when possible.
    #[arg(long)]
    pub(crate) fix: bool,
}

/// Parses and runs one command.
pub(crate) fn run<I, T>(argv: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(error) => {
            let informational = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            let _ = error.print();
            return if informational {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    let project = cli.project.clone().unwrap_or_else(|| PathBuf::from("."));
    match &cli.command {
        Command::New(args) => new::run(&cli, args),
        Command::Init(args) => init::run(&cli, args),
        Command::Build(args) => build::build(&cli, &project, args),
        Command::Run(args) => run::run(&cli, &project, args),
        Command::Check(args) => check::check(&cli, &project, args),
        Command::Metadata(args) => metadata::metadata(&cli, &project, args),
        Command::Doctor(args) => doctor::doctor(&cli, &project, args),
    }
}

/// Loads the project, mapping failures to a stable exit status.
pub(crate) fn load(project: &std::path::Path) -> Result<seal_project::Project, ExitCode> {
    seal_project::Project::load(project).map_err(|error| ui::error(error.code(), error.message()))
}
