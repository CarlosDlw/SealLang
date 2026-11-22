//! Argument parsing and command dispatch.

mod add;
mod affected;
mod api;
mod audit;
pub(crate) mod bench;
pub(crate) mod build;
mod cache;
mod check;
mod clean;
mod completions;
mod config;
mod dependency;
mod doc;
mod doctor;
mod graph;
mod harness;
mod info;
mod init;
mod lint;
mod login;
mod logout;
pub(crate) mod metadata;
pub(crate) mod new;
mod owner;
mod publish;
mod registry;
pub(crate) mod remote;
mod remove;
mod run;
mod sbom;
mod search;
mod semver_check;
mod snapshot;
mod stdlib;
mod target;
mod task;
mod test;
mod toolchain;
mod tree;
mod update;
mod vendor;
mod verify_repro;
mod watch;
mod why;
mod yank;

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
    /// A freestanding kernel with a linker script and `_start`.
    Kernel,
    /// A multi-file compiler pipeline with an AST, parser, and evaluator.
    Compiler,
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

    /// Forbid network access while loading registry dependencies.
    #[arg(long, global = true)]
    pub(crate) offline: bool,

    /// Resolve the lowest compatible registry versions.
    #[arg(long = "minimal-versions", global = true)]
    pub(crate) minimal_versions: bool,

    /// Resolve the newest compatible registry versions (the default).
    #[arg(long, global = true, conflicts_with = "minimal_versions")]
    pub(crate) latest: bool,

    /// Use only the vendor tree for registry dependencies.
    #[arg(long = "vendor", global = true)]
    pub(crate) vendor: bool,

    /// Use a REAPI v2 remote cache endpoint.
    #[arg(long, global = true, value_name = "URL")]
    pub(crate) remote: Option<String>,

    /// Execute compiler actions on the remote endpoint.
    #[arg(long = "remote-execution", global = true)]
    pub(crate) remote_execution: bool,

    /// Fail instead of falling back when the remote endpoint is unavailable.
    #[arg(long = "require-remote", global = true)]
    pub(crate) require_remote: bool,

    /// The REAPI instance name.
    #[arg(long = "remote-instance", global = true, value_name = "NAME")]
    pub(crate) remote_instance: Option<String>,

    /// The remote request timeout in seconds.
    #[arg(long = "remote-timeout", global = true, value_name = "SECONDS")]
    pub(crate) remote_timeout: Option<u64>,

    /// The number of transient remote request retries.
    #[arg(long = "remote-retries", global = true, value_name = "N")]
    pub(crate) remote_retries: Option<u32>,

    /// A PEM certificate authority bundle for the remote endpoint.
    #[arg(long = "remote-ca-cert", global = true, value_name = "PATH")]
    pub(crate) remote_ca_cert: Option<PathBuf>,

    /// The TLS server name when it differs from the URL host.
    #[arg(long = "remote-tls-domain", global = true, value_name = "NAME")]
    pub(crate) remote_tls_domain: Option<String>,

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
    /// Build and run the test targets of the selected packages.
    Test(TestArgs),
    /// Build and run benchmark targets.
    Bench(BenchArgs),
    /// Rebuild, check, or test whenever a source file changes.
    Watch(WatchArgs),
    /// Print the resolved configuration and where each value came from.
    Config(ConfigArgs),
    /// Generate shell completions.
    Completions(CompletionsArgs),
    /// Generate a manual page.
    Man(ManArgs),
    /// Run the compiler lint policy over the selected targets.
    Lint(LintArgs),
    /// Print the resolved package-instance graph.
    Tree(QueryArgs),
    /// Explain why packages and features are in the plan.
    Why(WhyArgs),
    /// Print a package, target, action, or module graph.
    Graph(GraphArgs),
    /// List packages and targets affected by changes since a revision.
    Affected(AffectedArgs),
    /// Print the project model as JSON.
    Metadata(MetadataArgs),
    /// Inspect and verify the installed standard library.
    Stdlib(stdlib::StdlibArgs),
    /// Check the toolchain, linker, cache, and lockfile.
    Doctor(DoctorArgs),
    /// Remove build outputs and the local cache.
    Clean,
    /// Inspect, verify, prune, or explain the local cache.
    Cache(CacheArgs),
    /// Run a declarative task from `seal.toml`.
    Task(TaskArgs),
    /// Build in two directories and compare every artifact byte.
    VerifyRepro(verify_repro::VerifyReproArgs),
    /// Inspect, install, pin, and verify toolchains.
    Toolchain(toolchain::ToolchainArgs),
    /// Inspect and install TOML target specifications.
    Target(target::TargetArgs),
    /// Add a registry dependency and update the lockfile.
    Add(AddArgs),
    /// Remove a dependency and update the lockfile.
    Remove(RemoveArgs),
    /// Re-resolve registry dependencies.
    Update(UpdateArgs),
    /// Copy registry dependencies into `vendor/`.
    Vendor(VendorArgs),
    /// Audit locked dependencies.
    Audit(AuditArgs),
    /// Audit locked dependencies; an alias for `audit`.
    Deny(AuditArgs),
    /// Generate a CycloneDX SBOM.
    Sbom(SbomArgs),
    /// Search the registry catalog.
    Search(SearchArgs),
    /// Show every version and dependency of one package.
    Info(InfoArgs),
    /// Package and publish one package or a whole workspace.
    Publish(PublishArgs),
    /// Yank or restore a published version.
    Yank(YankArgs),
    /// Claim a namespace and manage its owners.
    Owner(OwnerArgs),
    /// Store a registry token in the credentials file.
    Login(LoginArgs),
    /// Remove a stored registry token.
    Logout(LogoutArgs),
    /// Print the public API snapshot of a package.
    Api(ApiArgs),
    /// Compare a package API against a baseline version or file.
    SemverCheck(SemverCheckArgs),
    /// Generate HTML or JSON documentation for a package.
    Doc(DocArgs),
}

/// Options shared by build-like commands.
#[derive(Clone, Debug, Args)]
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

    /// Exclude a package from the selection.
    #[arg(long, value_name = "PACKAGE")]
    pub(crate) exclude: Vec<String>,

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
    #[arg(long, alias = "target", value_name = "TRIPLE")]
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

    /// Force a boolean `cfg` fact (`test`, `bench`, `debug_assertions`).
    #[arg(long = "cfg", value_name = "FACT")]
    pub(crate) cfgs: Vec<String>,

    /// Disable the implicit standard library prelude.
    #[arg(long = "no-prelude")]
    pub(crate) no_prelude: bool,

    /// Fail if the lockfile would change.
    #[arg(long)]
    pub(crate) locked: bool,

    /// Fail if the lockfile would change and forbid network access.
    #[arg(long)]
    pub(crate) frozen: bool,
}

impl Selection {
    /// A selection with no filters, used by dependency commands.
    pub(crate) fn default_for_command() -> Self {
        Self {
            targets: Vec::new(),
            package: None,
            workspace: false,
            exclude: Vec::new(),
            all_targets: false,
            profile: None,
            release: false,
            target_triple: None,
            features: Vec::new(),
            all_features: false,
            no_default_features: false,
            no_prelude: false,
            cfgs: Vec::new(),
            locked: false,
            frozen: false,
        }
    }
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

    /// Number of actions to run in parallel.
    #[arg(short = 'j', long, value_name = "N")]
    pub(crate) jobs: Option<usize>,

    /// Print one timing line per action.
    #[arg(long)]
    pub(crate) timings: bool,

    /// Bypass the action cache for this build.
    #[arg(long)]
    pub(crate) no_cache: bool,
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

/// Report format for `seal test` and `seal bench`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum HarnessFormat {
    /// One line per test plus a summary.
    Human,
    /// Newline-delimited JSON events.
    Json,
    /// A JUnit XML document.
    Junit,
}

/// `seal test`.
#[derive(Debug, Args)]
pub(crate) struct TestArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// Keep tests whose full name contains this text; repeatable.
    #[arg(long = "filter", value_name = "TEXT")]
    pub(crate) filters: Vec<String>,

    /// Require an exact match with a `--filter` value.
    #[arg(long)]
    pub(crate) exact: bool,

    /// Skip tests whose full name contains this text; repeatable.
    #[arg(long = "skip", value_name = "TEXT")]
    pub(crate) skips: Vec<String>,

    /// Run one shard, written `INDEX/TOTAL` or `hash:INDEX/TOTAL`.
    #[arg(long, value_name = "SHARD")]
    pub(crate) partition: Option<String>,

    /// Repeat every selected test this many times.
    #[arg(long, default_value = "1", value_name = "N")]
    pub(crate) stress: u32,

    /// Retry a failed test up to this many extra times.
    #[arg(long, default_value = "0", value_name = "N")]
    pub(crate) retries: u32,

    /// Kill a test after this many seconds; zero disables the timeout.
    #[arg(long, default_value = "60", value_name = "SECONDS")]
    pub(crate) timeout: u64,

    /// Keep running after the first failure.
    #[arg(long)]
    pub(crate) no_fail_fast: bool,

    /// Number of tests to run in parallel.
    #[arg(short = 'j', long, value_name = "N")]
    pub(crate) jobs: Option<usize>,

    /// Print the selected tests without running them.
    #[arg(long)]
    pub(crate) list: bool,

    /// Report format.
    #[arg(long, value_enum, default_value = "human")]
    pub(crate) format: HarnessFormat,

    /// Write the report to this path instead of stdout.
    #[arg(short = 'o', long, value_name = "PATH", requires = "format")]
    pub(crate) output: Option<PathBuf>,

    /// Bypass the action cache for this build.
    #[arg(long)]
    pub(crate) no_cache: bool,
}

/// `seal bench`.
#[derive(Debug, Args)]
pub(crate) struct BenchArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// Keep benchmarks whose full name contains this text; repeatable.
    #[arg(long = "filter", value_name = "TEXT")]
    pub(crate) filters: Vec<String>,

    /// Require an exact match with a `--filter` value.
    #[arg(long)]
    pub(crate) exact: bool,

    /// Skip benchmarks whose full name contains this text.
    #[arg(long = "skip", value_name = "TEXT")]
    pub(crate) skips: Vec<String>,

    /// Run one shard, written `INDEX/TOTAL` or `hash:INDEX/TOTAL`.
    #[arg(long, value_name = "SHARD")]
    pub(crate) partition: Option<String>,

    /// Iterations of the benchmark body per process.
    #[arg(long, default_value = "100", value_name = "N")]
    pub(crate) iterations: u32,

    /// Number of measured processes.
    #[arg(long, default_value = "3", value_name = "N")]
    pub(crate) samples: u32,

    /// Kill one sample after this many seconds.
    #[arg(long, default_value = "120", value_name = "SECONDS")]
    pub(crate) timeout: u64,

    /// Number of benchmarks to run in parallel.
    #[arg(short = 'j', long, value_name = "N")]
    pub(crate) jobs: Option<usize>,

    /// Print the selected benchmarks without running them.
    #[arg(long)]
    pub(crate) list: bool,

    /// Compare against a baseline written by `--save`.
    #[arg(long, value_name = "PATH")]
    pub(crate) baseline: Option<PathBuf>,

    /// Write the measured results as a baseline.
    #[arg(long, value_name = "PATH")]
    pub(crate) save: Option<PathBuf>,

    /// Fail when a measurement is slower than the baseline by this percentage.
    #[arg(long, default_value = "20", value_name = "PERCENT")]
    pub(crate) threshold: f64,

    /// Print the results as JSON.
    #[arg(long)]
    pub(crate) json: bool,

    /// Bypass the action cache for this build.
    #[arg(long)]
    pub(crate) no_cache: bool,
}

/// The command a watcher runs after a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum WatchAction {
    /// Rebuild the selected targets.
    Build,
    /// Type-check the selected targets.
    Check,
    /// Run the selected tests.
    Test,
}

/// `seal watch`.
#[derive(Debug, Args)]
pub(crate) struct WatchArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// The command to repeat.
    #[arg(long, value_enum, default_value = "build")]
    pub(crate) action: WatchAction,

    /// Wait this many milliseconds after the last change.
    #[arg(long, default_value = "200", value_name = "MILLISECONDS")]
    pub(crate) debounce: u64,

    /// Run once and exit; useful in scripts and tests.
    #[arg(long)]
    pub(crate) once: bool,

    /// Keep tests whose full name contains this text.
    #[arg(long = "filter", value_name = "TEXT")]
    pub(crate) filters: Vec<String>,
}

/// `seal config`.
#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    /// Print only this key.
    #[arg(long, value_name = "KEY")]
    pub(crate) key: Option<String>,

    /// List every resolved key.
    #[arg(long)]
    pub(crate) list: bool,

    /// Print a JSON array of `{key, value, origin}` objects.
    #[arg(long)]
    pub(crate) json: bool,

    /// Print only the origin of each value.
    #[arg(long)]
    pub(crate) origin: bool,

    /// Override the target triple for this resolution.
    #[arg(long, value_name = "TRIPLE")]
    pub(crate) target: Option<String>,

    /// Override the profile for this resolution.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,

    /// Override the job count for this resolution.
    #[arg(long, value_name = "N")]
    pub(crate) jobs: Option<usize>,
}

/// Shells accepted by `seal completions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Shell {
    /// Bourne Again Shell.
    Bash,
    /// Z shell.
    Zsh,
    /// Friendly Interactive Shell.
    Fish,
    /// PowerShell; `pwsh` is accepted as an alias.
    #[value(alias = "pwsh")]
    Powershell,
}

/// `seal completions`.
#[derive(Debug, Args)]
pub(crate) struct CompletionsArgs {
    /// Shell to generate completions for.
    pub(crate) shell: Shell,

    /// Write to this path instead of stdout.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,
}

/// `seal man`.
#[derive(Debug, Args)]
pub(crate) struct ManArgs {
    /// Write to this path instead of stdout.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,
}

/// `seal lint`.
#[derive(Debug, Args)]
pub(crate) struct LintArgs {
    #[command(flatten)]
    pub(crate) selection: Selection,

    /// Treat a lint as an error; `warnings` covers every compiler warning.
    #[arg(long, value_name = "LINT")]
    pub(crate) deny: Vec<String>,

    /// Allow a lint that the compiler denies by default.
    #[arg(long, value_name = "LINT")]
    pub(crate) allow: Vec<String>,

    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Options shared by `tree` and `why`.
#[derive(Clone, Debug, Args)]
pub(crate) struct QueryArgs {
    /// Select one package by name.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,

    /// Compile for a target triple.
    #[arg(long, alias = "target", value_name = "TRIPLE")]
    pub(crate) target_triple: Option<String>,

    /// Enable features.
    #[arg(long, value_delimiter = ',', value_name = "FEATURE")]
    pub(crate) features: Vec<String>,

    /// Enable every feature.
    #[arg(long, conflicts_with = "features")]
    pub(crate) all_features: bool,

    /// Disable default features.
    #[arg(long)]
    pub(crate) no_default_features: bool,

    /// Force a boolean `cfg` fact (`test`, `bench`, `debug_assertions`).
    #[arg(long = "cfg", value_name = "FACT")]
    pub(crate) cfgs: Vec<String>,

    /// Show packages that resolve to more than one version.
    #[arg(long)]
    pub(crate) duplicates: bool,
}

/// Options for `seal why`.
#[derive(Debug, Args)]
pub(crate) struct WhyArgs {
    /// Select one package by name.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,

    /// Compile for a target triple.
    #[arg(long = "target-triple", value_name = "TRIPLE")]
    pub(crate) target_triple: Option<String>,

    /// Enable features.
    #[arg(long, value_delimiter = ',', value_name = "FEATURE")]
    pub(crate) features: Vec<String>,

    /// Enable every feature.
    #[arg(long, conflicts_with = "features")]
    pub(crate) all_features: bool,

    /// Disable default features.
    #[arg(long)]
    pub(crate) no_default_features: bool,

    /// Force a boolean `cfg` fact (`test`, `bench`, `debug_assertions`).
    #[arg(long = "cfg", value_name = "FACT")]
    pub(crate) cfgs: Vec<String>,

    /// Explain why a feature is enabled.
    #[arg(long, value_name = "FEATURE")]
    pub(crate) feature: Option<String>,

    /// Explain why a source file is compiled.
    #[arg(long, value_name = "PATH")]
    pub(crate) file: Option<String>,

    /// Explain why a target is built.
    #[arg(long, value_name = "TARGET")]
    pub(crate) target: Option<String>,

    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,
}

/// `seal graph`.
#[derive(Debug, Args)]
pub(crate) struct GraphArgs {
    /// The graph kind: packages, targets, actions, or modules.
    #[arg(value_name = "KIND", default_value = "packages")]
    pub(crate) kind: String,

    /// Output format.
    #[arg(long, value_enum, default_value = "ascii")]
    pub(crate) format: GraphFormat,

    /// Restrict the graph to a node and its descendants.
    #[arg(long, value_name = "NODE")]
    pub(crate) focus: Option<String>,

    /// Follow edges in the reverse direction.
    #[arg(long)]
    pub(crate) reverse: bool,
}

/// Graph output formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum GraphFormat {
    /// A human-readable tree.
    Ascii,
    /// Graphviz DOT.
    Dot,
    /// Mermaid flowchart.
    Mermaid,
    /// Versioned JSON.
    Json,
}

/// `seal affected`.
#[derive(Debug, Args)]
pub(crate) struct AffectedArgs {
    /// The revision to compare the working tree against.
    #[arg(long, value_name = "REV")]
    pub(crate) since: String,

    /// Check every affected target.
    #[arg(long)]
    pub(crate) run: bool,

    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,

    /// Compile for a target triple.
    #[arg(long, alias = "target", value_name = "TRIPLE")]
    pub(crate) target_triple: Option<String>,

    /// Enable features.
    #[arg(long, value_delimiter = ',', value_name = "FEATURE")]
    pub(crate) features: Vec<String>,

    /// Enable every feature.
    #[arg(long, conflicts_with = "features")]
    pub(crate) all_features: bool,

    /// Disable default features.
    #[arg(long)]
    pub(crate) no_default_features: bool,
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

/// `seal cache`.
#[derive(Debug, Args)]
pub(crate) struct CacheArgs {
    /// The cache operation.
    #[command(subcommand)]
    pub(crate) command: CacheCommand,
}

/// Cache operations.
#[derive(Debug, Subcommand)]
pub(crate) enum CacheCommand {
    /// Print object and action-record statistics.
    Stats,
    /// Verify every object and action record.
    Verify,
    /// Remove least-recently-used objects and old records.
    Prune {
        /// Maximum object bytes to keep.
        #[arg(long, value_name = "BYTES")]
        max_size: Option<u64>,
        /// Remove action records older than this many seconds.
        #[arg(long, value_name = "SECONDS")]
        older_than: Option<u64>,
    },
    /// Print the inputs, outputs, and command of one action key.
    Explain {
        /// The action key.
        key: String,
    },
}

/// `seal add`.
#[derive(Debug, Args)]
pub(crate) struct AddArgs {
    /// A dependency such as `foo`, `foo@1.2`, or `@scope/foo@^2`.
    pub(crate) spec: String,
    /// Enable these features on the dependency.
    #[arg(long, value_delimiter = ',', value_name = "FEATURE")]
    pub(crate) features: Vec<String>,
    /// Mark the dependency optional.
    #[arg(long)]
    pub(crate) optional: bool,
    /// Disable the dependency's default feature.
    #[arg(long)]
    pub(crate) no_default_features: bool,
    /// The package whose manifest to edit.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// Print the proposed change without writing anything.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Apply the change without asking for confirmation.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `seal remove`.
#[derive(Debug, Args)]
pub(crate) struct RemoveArgs {
    /// The dependency alias to remove.
    pub(crate) name: String,
    /// The package whose manifest to edit.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// Print the proposed change without writing anything.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Apply the change without asking for confirmation.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `seal update`.
#[derive(Debug, Args)]
pub(crate) struct UpdateArgs {
    /// Packages to update; empty updates every registry dependency.
    #[arg(value_name = "PACKAGE")]
    pub(crate) packages: Vec<String>,
    /// Pin a package instead of selecting the latest compatible version.
    #[arg(long, value_name = "PACKAGE@VERSION")]
    pub(crate) precise: Vec<String>,
    /// Print the proposed selection without writing the lockfile.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

/// `seal vendor`.
#[derive(Debug, Args)]
pub(crate) struct VendorArgs {
    /// Remove the vendor tree and recreate it.
    #[arg(long)]
    pub(crate) clean: bool,
}

/// `seal audit` and `seal deny`.
#[derive(Debug, Args)]
pub(crate) struct AuditArgs {
    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,
    /// Require and verify signed provenance, SBOM, API snapshots, and log inclusion.
    #[arg(long)]
    pub(crate) require_provenance: bool,
}

/// `seal publish`.
#[derive(Debug, Args)]
pub(crate) struct PublishArgs {
    /// Publish one package by name.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// Publish every package in dependency order.
    #[arg(long, conflicts_with = "package")]
    pub(crate) workspace: bool,
    /// Build and print the publication without contacting the registry.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Publish even when the Git working tree has changes.
    #[arg(long)]
    pub(crate) allow_dirty: bool,
    /// A file containing a 64-character hexadecimal Ed25519 seed.
    #[arg(long, value_name = "PATH")]
    pub(crate) signing_key: Option<PathBuf>,
    /// The OIDC identity to claim in the provenance statement.
    #[arg(long, value_name = "IDENTITY")]
    pub(crate) identity: Option<String>,
}

/// `seal yank`.
#[derive(Debug, Args)]
pub(crate) struct YankArgs {
    /// The version, written `name@version`.
    pub(crate) spec: String,
    /// Restore a yanked version instead of yanking it.
    #[arg(long)]
    pub(crate) undo: bool,
}

/// `seal owner`.
#[derive(Debug, Args)]
pub(crate) struct OwnerArgs {
    /// The namespace, such as `@acme` or a preapproved unscoped name.
    pub(crate) namespace: String,
    /// List the current owners.
    #[arg(long, conflicts_with_all = ["add", "remove", "claim"])]
    pub(crate) list: bool,
    /// Add an owner identity.
    #[arg(long, value_name = "IDENTITY", conflicts_with_all = ["remove", "claim"])]
    pub(crate) add: Option<String>,
    /// Remove an owner identity.
    #[arg(long, value_name = "IDENTITY", conflicts_with_all = ["claim"])]
    pub(crate) remove: Option<String>,
    /// Claim the namespace with a proof.
    #[arg(long, conflicts_with_all = ["list", "add", "remove"])]
    pub(crate) claim: bool,
    /// The proof kind: `config`, `org`, `oidc`, or `dns`.
    #[arg(long, value_name = "KIND", requires = "claim")]
    pub(crate) proof: Option<String>,
    /// The DNS verification token or OIDC subject.
    #[arg(long, value_name = "VALUE", requires = "claim")]
    pub(crate) proof_value: Option<String>,
}

/// `seal login`.
#[derive(Debug, Args)]
pub(crate) struct LoginArgs {
    /// Override the registry URL.
    #[arg(long, value_name = "URL")]
    pub(crate) registry: Option<String>,
    /// Store the token under this registry name instead of the configured one.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: Option<String>,
    /// Accept a plain `http://` URL for a local registry.
    #[arg(long)]
    pub(crate) allow_insecure: bool,
    /// Read the token from stdin instead of an environment variable.
    #[arg(long)]
    pub(crate) token_stdin: bool,
}

/// `seal logout`.
#[derive(Debug, Args)]
pub(crate) struct LogoutArgs {
    /// Remove the token stored under this registry name.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: Option<String>,
}

/// `seal api`.
#[derive(Debug, Args)]
pub(crate) struct ApiArgs {
    /// The package to inspect; defaults to the current package.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// Print the snapshot JSON instead of a summary.
    #[arg(long)]
    pub(crate) json: bool,
}

/// `seal semver-check`.
#[derive(Debug, Args)]
pub(crate) struct SemverCheckArgs {
    /// A baseline version or an API snapshot file.
    pub(crate) baseline: String,
    /// The package to inspect; defaults to the current package.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// Print a JSON report.
    #[arg(long)]
    pub(crate) json: bool,
}

/// `seal doc`.
#[derive(Debug, Args)]
pub(crate) struct DocArgs {
    /// The package to document; defaults to the current package.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    pub(crate) package: Option<String>,

    /// Output directory for HTML.
    #[arg(short = 'o', long, value_name = "DIR", default_value = "target/doc")]
    pub(crate) output: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value = "html")]
    pub(crate) format: DocFormat,

    /// Print the documentation JSON to stdout.
    #[arg(long)]
    pub(crate) json: bool,

    /// Open the generated index in the default browser.
    #[arg(long)]
    pub(crate) open: bool,

    /// Serve the generated documentation over HTTP.
    #[arg(long)]
    pub(crate) serve: bool,

    /// Address used by `--serve`.
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:8000")]
    pub(crate) address: String,
}

/// Documentation output formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum DocFormat {
    /// A browsable HTML tree.
    Html,
    /// A versioned JSON document.
    Json,
}

/// `seal sbom`.
#[derive(Debug, Args)]
pub(crate) struct SbomArgs {
    /// Write the JSON document to this path instead of stdout.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,
}

/// `seal search`.
#[derive(Debug, Args)]
pub(crate) struct SearchArgs {
    /// Text matched against package names.
    pub(crate) query: String,
    /// Print JSON.
    #[arg(long)]
    pub(crate) json: bool,
    /// Maximum results.
    #[arg(long, default_value = "20")]
    pub(crate) limit: usize,
}

/// `seal info`.
#[derive(Debug, Args)]
pub(crate) struct InfoArgs {
    /// The package name.
    pub(crate) name: String,
    /// Print JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

/// `seal task`.
#[derive(Debug, Args)]
pub(crate) struct TaskArgs {
    /// The task name.
    pub(crate) name: String,

    /// Arguments appended to every step.
    #[arg(last = true, value_name = "ARGS")]
    pub(crate) args: Vec<String>,

    /// Continue after a failing step.
    #[arg(long)]
    pub(crate) no_fail_fast: bool,
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
        Command::Test(args) => test::test(&cli, &project, args),
        Command::Bench(args) => bench::bench(&cli, &project, args),
        Command::Watch(args) => watch::watch(&cli, &project, args),
        Command::Config(args) => config::run(&cli, &project, args),
        Command::Completions(args) => completions::completions(args),
        Command::Man(args) => completions::man(args),
        Command::Tree(args) => tree::tree(&cli, &project, args),
        Command::Why(args) => why::why(&cli, &project, args),
        Command::Graph(args) => graph::graph(&cli, &project, args),
        Command::Affected(args) => affected::affected(&cli, &project, args),
        Command::Metadata(args) => metadata::metadata(&cli, &project, args),
        Command::Stdlib(args) => stdlib::run(args),
        Command::Doctor(args) => doctor::doctor(&cli, &project, args),
        Command::Clean => clean::clean(&cli, &project),
        Command::Cache(args) => cache::cache(&cli, &project, args),
        Command::Task(args) => task::task(&cli, &project, args),
        Command::VerifyRepro(args) => verify_repro::verify_repro(&cli, &project, args),
        Command::Toolchain(args) => toolchain::run(&cli, &project, args),
        Command::Target(args) => target::run(&cli, &project, args),
        Command::Add(args) => add::run(&cli, &project, args),
        Command::Remove(args) => remove::run(&cli, &project, args),
        Command::Update(args) => update::run(&cli, &project, args),
        Command::Vendor(args) => vendor::run(&cli, &project, args),
        Command::Audit(args) | Command::Deny(args) => audit::run(&cli, &project, args),
        Command::Sbom(args) => sbom::run(&cli, &project, args),
        Command::Search(args) => search::run(&cli, &project, args),
        Command::Info(args) => info::run(&cli, &project, args),
        Command::Publish(args) => publish::run(&cli, &project, args),
        Command::Yank(args) => yank::run(&cli, &project, args),
        Command::Owner(args) => owner::run(&cli, &project, args),
        Command::Login(args) => login::run(&cli, &project, args),
        Command::Logout(args) => logout::run(&cli, &project, args),
        Command::Api(args) => api::run(&cli, &project, args),
        Command::SemverCheck(args) => semver_check::run(&cli, &project, args),
        Command::Doc(args) => doc::run(&cli, &project, args),
        Command::Lint(args) => lint::lint(&cli, &project, args),
    }
}

/// Builds project-loading options from global flags.
pub(crate) fn project_options(cli: &Cli) -> seal_project::ProjectOptions {
    seal_project::ProjectOptions {
        offline: cli.offline,
        minimal_versions: cli.minimal_versions,
        update: false,
        vendor_only: cli.vendor,
        update_packages: Vec::new(),
        pinned: std::collections::BTreeMap::new(),
    }
}

/// Loads the project, mapping failures to a stable exit status.
pub(crate) fn load(
    cli: &Cli,
    project: &std::path::Path,
) -> Result<seal_project::Project, ExitCode> {
    load_with(cli, project, &project_options(cli))
}

/// Loads the project for a build-like command, including `--frozen`.
pub(crate) fn load_for_selection(
    cli: &Cli,
    project: &std::path::Path,
    selection: &Selection,
) -> Result<seal_project::Project, ExitCode> {
    let mut options = project_options(cli);
    options.offline |= selection.frozen;
    load_with(cli, project, &options)
}

/// Loads the project with explicit registry options.
pub(crate) fn load_with(
    cli: &Cli,
    project: &std::path::Path,
    options: &seal_project::ProjectOptions,
) -> Result<seal_project::Project, ExitCode> {
    let _ = cli;
    seal_project::Project::load_with(project, options)
        .map_err(|error| ui::error(error.code(), error.message()))
}
