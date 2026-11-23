//! Driving every compiler phase for each requested input.
//!
//! Each phase runs only when the previous one reported no errors: later phases
//! assume the invariants their predecessors establish. The control flow is
//! written as early returns so the order of those checks stays obvious.
//!
//! The request type makes the difference between building and checking
//! structural: only a build carries an artifact to write, so no command can
//! accidentally produce files or silently skip code generation.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::sync::Arc;

use seal_codegen::{CodegenInput, CodegenOptions, Emit};
use seal_def::{DefId, PackageId, PackageInstanceId};
use seal_index::Idx;
use seal_metadata::read_archive_file;
use seal_plan::CompilePlan;
use seal_session::Session;
use seal_target::host_triple;

use crate::api;
use crate::body_cache;
use crate::cli::plan::resolve_entry;
use crate::frontend::Lowered;

use crate::cli::{BuildArgs, CheckArgs, CompileArgs, EmitArg, LinkArgs, OutputArgs, RunArgs};
use crate::options::{session_options, validate_lints};
use crate::output::artifact_path;
use crate::report;
use crate::temp::TemporaryDirectory;

/// What the selected command asks the pipeline to do.
#[derive(Clone)]
enum Request<'a> {
    /// Run analysis through monomorphization and write nothing.
    Check,
    /// Run the whole pipeline and write an artifact.
    Build {
        emit: Emit,
        output: &'a OutputArgs,
        runtime_archive: Option<&'a Path>,
        link: &'a LinkArgs,
        planned_output: Option<PathBuf>,
        link_metadata: Option<&'a Path>,
        cgu_count: Option<u32>,
    },
    /// Write a JSON API snapshot after analysis.
    Api { path: PathBuf },
}

/// Project-level settings that accompany a lowered HIR body.
struct HirRequest<'a> {
    /// The primary source input.
    input: &'a Path,
    /// The selected entry definition, when the plan names one.
    entry: Option<DefId>,
    /// The plan profile, when compiling a project target.
    profile: Option<&'a seal_plan::PlanProfile>,
    /// The complete plan, when compiling a project target.
    plan: Option<&'a CompilePlan>,
}

/// Whether compilation completed without errors.
enum Outcome {
    /// All inputs compiled successfully.
    Success,
    /// Compilation reported errors and produced no usable artifact.
    Failed(ExitCode),
}

impl Outcome {
    /// Returns the process status a build or check should report.
    fn code(self) -> ExitCode {
        match self {
            Self::Success => ExitCode::SUCCESS,
            Self::Failed(code) => code,
        }
    }
}

impl Request<'_> {
    /// Replaces the artifact path with the one named by a compile plan.
    fn with_planned_output(mut self, path: Option<PathBuf>) -> Self {
        if let Self::Build { planned_output, .. } = &mut self {
            *planned_output = path;
        }
        self
    }

    /// Returns the output options that affect session configuration, if any.
    fn output(&self) -> Option<&OutputArgs> {
        match self {
            Self::Check | Self::Api { .. } => None,
            Self::Build { output, .. } => Some(output),
        }
    }

    /// Returns the verb that describes this request in the failure summary.
    fn action(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build { .. } => "compile",
            Self::Api { .. } => "snapshot",
        }
    }
}

/// Compiles every input and writes the requested artifacts.
pub(crate) fn build(args: &BuildArgs) -> Result<ExitCode, String> {
    let request = if args.output.is_api() {
        Request::Api {
            path: args
                .output
                .path
                .clone()
                .unwrap_or_else(|| PathBuf::from("api.json")),
        }
    } else {
        Request::Build {
            emit: args.output.artifact(),
            output: &args.output,
            runtime_archive: args.runtime_archive.as_deref(),
            link: &args.link,
            planned_output: None,
            link_metadata: args.link_metadata.as_deref(),
            cgu_count: args.cgu,
        }
    };
    compile(&args.compile, request).map(Outcome::code)
}

/// Builds one input into a temporary executable and runs it.
///
/// The executable is removed when this function returns, whether compilation
/// or execution succeeds or fails. Standard streams are inherited so the
/// program behaves as it would when launched from a shell.
pub(crate) fn run(args: &RunArgs) -> Result<ExitCode, String> {
    ensure_host_target(args)?;
    let temporary = TemporaryDirectory::new()?;
    let executable = temporary.executable();
    let output = OutputArgs {
        emit: EmitArg::Exe,
        path: Some(executable.clone()),
        optimization: args.optimization.clone(),
    };
    let compile_arguments = CompileArgs {
        inputs: vec![args.input.clone()],
        target: args.target.clone(),
        diagnostics: args.diagnostics.clone(),
        plan: false,
        plan_file: None,
        expose: Vec::new(),
        core_only: false,
        no_stdlib: false,
        no_prelude: false,
    };
    let request = Request::Build {
        emit: Emit::Executable,
        output: &output,
        runtime_archive: args.runtime_archive.as_deref(),
        link: &args.link,
        planned_output: None,
        link_metadata: None,
        cgu_count: None,
    };

    match compile(&compile_arguments, request)? {
        Outcome::Failed(code) => Ok(code),
        Outcome::Success => execute(&executable, &args.program_args),
    }
}

/// Rejects execution of a binary built for a different architecture or OS.
fn ensure_host_target(args: &RunArgs) -> Result<(), String> {
    let Some(target) = &args.target.target else {
        return Ok(());
    };
    let host = host_triple();
    if target.arch() == host.arch() && target.os() == host.os() {
        return Ok(());
    }
    Err(format!(
        "`run` can only execute host programs; build for `{}` with `sealc build` and run the executable separately",
        target
    ))
}

/// Executes the built program and maps its termination status to `sealc`'s.
fn execute(executable: &Path, arguments: &[OsString]) -> Result<ExitCode, String> {
    let status = Command::new(executable)
        .args(arguments)
        .status()
        .map_err(|error| format!("cannot execute `{}`: {error}", executable.display()))?;
    program_exit_code(status)
}

/// Converts the child status into the status `sealc` returns to its caller.
#[cfg(unix)]
fn program_exit_code(status: ExitStatus) -> Result<ExitCode, String> {
    use std::os::unix::process::ExitStatusExt;

    if let Some(code) = status.code() {
        return Ok(ExitCode::from(code as u8));
    }
    if let Some(signal) = status.signal() {
        // Shells report a signal-terminated child as 128 + the signal number.
        return Ok(ExitCode::from((128 + signal) as u8));
    }
    Err("the program terminated without an exit status".to_owned())
}

/// Converts the child status into the status `sealc` returns to its caller.
#[cfg(not(unix))]
fn program_exit_code(status: ExitStatus) -> Result<ExitCode, String> {
    status
        .code()
        .map(|code| ExitCode::from(code as u8))
        .ok_or_else(|| "the program terminated without an exit status".to_owned())
}

/// Generates one object from a `.sealmono` unit without lowering again.
pub(crate) fn codegen(args: &crate::cli::CodegenArgs) -> Result<ExitCode, String> {
    let archive = match read_archive_file(&args.mono) {
        Ok(archive) => archive,
        Err(error) => {
            eprintln!("error: {error}");
            return Ok(ExitCode::from(1));
        }
    };
    let manifest = archive.manifest();
    let Some(bitcode) = archive.module(args.cgu) else {
        eprintln!(
            "error: E8070: `{}` has no code generation unit {}; it declares {}",
            args.mono.display(),
            args.cgu,
            manifest.cgu_count()
        );
        return Ok(ExitCode::from(1));
    };
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| args.mono.with_extension(format!("{}.o", args.cgu)));
    if let Err(error) = seal_codegen::emit_object_from_bitcode(
        bitcode,
        &output,
        &manifest.target,
        manifest.profile.opt_level,
    ) {
        eprintln!("error: {error}");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// Analyzes every input without writing anything.
pub(crate) fn check(args: &CheckArgs) -> Result<ExitCode, String> {
    compile(&args.compile, Request::Check).map(Outcome::code)
}

/// Creates the session, compiles each input, and reports the final status.
fn compile(arguments: &CompileArgs, request: Request<'_>) -> Result<Outcome, String> {
    validate_lints(arguments)?;
    let plan = match &arguments.plan_file {
        Some(path) => match CompilePlan::load(path) {
            Ok(plan) => Some(plan),
            Err(error) => {
                eprintln!("error: {error}");
                return Ok(Outcome::Failed(ExitCode::from(1)));
            }
        },
        None => None,
    };
    let mut options = session_options(arguments, request.output());
    if let Some(plan) = &plan {
        options.target =
            seal_target::TargetTriple::parse(&plan.target).map_err(|error| error.to_string())?;
        if let Some(target) = &plan.target_spec {
            let spec = target
                .to_spec()
                .map_err(|error| format!("invalid target specification: {error}"))?;
            options.target_spec = Some(spec);
        }
    }
    let session = crate::session_cache::acquire(&options)?;
    let session = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    session.reset_for_request();
    if let Some(plan) = plan {
        if !compile_plan_file(&session, request.clone(), plan, arguments.no_prelude) {
            return Ok(Outcome::Failed(ExitCode::from(1)));
        }
    } else if arguments.plan {
        compile_package(&session, request.clone(), arguments);
    } else {
        for input in &arguments.inputs {
            compile_input(&session, request.clone(), input, arguments);
        }
    }

    if let Some(error) = session.diag().emitter_error() {
        return Err(format!("failed to write diagnostics: {error}"));
    }
    if arguments.diagnostics.time_passes {
        eprint!("{}", session.profiler().report());
    }
    if session.abort_if_errors().is_err() {
        let files = if arguments.plan_file.is_some() {
            1
        } else {
            arguments.inputs.len()
        };
        let errors = session.diag().error_count();
        eprintln!(
            "error: could not {} {} input {} due to {} previous {}",
            request.action(),
            files,
            plural(files, "file", "files"),
            errors,
            plural(errors, "error", "errors"),
        );
        return Ok(Outcome::Failed(ExitCode::from(1)));
    }
    if arguments.diagnostics.deny_warnings {
        let warnings = session.diag().warning_count();
        if warnings > 0 {
            eprintln!(
                "error: {} {} denied by `--deny-warnings`",
                warnings,
                plural(warnings, "warning", "warnings")
            );
            return Ok(Outcome::Failed(ExitCode::from(1)));
        }
    }
    Ok(Outcome::Success)
}

/// Selects the singular or plural spelling of a counted noun.
const fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

/// Runs the whole pipeline for one input file.
fn compile_input(session: &Session, request: Request<'_>, input: &Path, arguments: &CompileArgs) {
    let identity = input.display().to_string();
    let Ok(file_id) = session.load_file_persistent(input, &identity) else {
        return;
    };
    let Some(file) = session.source_map().file(file_id) else {
        return;
    };
    let errors = session.diag().error_count();

    let Some(tokens) = session
        .profiler()
        .time("lex", || crate::frontend::lex(session, file_id, &file))
    else {
        return;
    };
    // A lexical error leaves `Error` tokens behind, and parsing them only adds
    // secondary syntax errors; the parser runs on a clean token stream.
    if session.diag().error_count() != errors {
        return;
    }

    let Some(ast) = session.profiler().time("parse", || {
        crate::frontend::parse(session, file_id, &file, &tokens)
    }) else {
        return;
    };
    // Lowering assumes a syntactically valid tree. A parse error already leaves
    // `Error` nodes behind, so the pipeline stops after `parse`.
    if session.diag().error_count() != errors {
        return;
    }
    let dependency = match crate::frontend::ast_dependency(session, file_id, &file) {
        Ok(dependency) => dependency,
        Err(error) => {
            session
                .diag()
                .bug(format!("the parse dependency is missing: {error}"));
        }
    };

    let injection = match inject_single(session, stdlib_mode(arguments), arguments.no_prelude) {
        Ok(injection) => injection,
        Err(message) => {
            eprintln!("error: {message}");
            return;
        }
    };
    let user = seal_hir::PackagePlanEntry {
        name: session.interner().intern("main"),
        edition: session.edition(),
        instance: PackageInstanceId::from_usize(0),
        files: vec![seal_hir::PackagePlanFile {
            file: file_id,
            primary: true,
        }],
        exposed: vec![seal_hir::ModulePath::root()],
        dependencies: injection.package_dependencies,
        cfg: seal_hir::CfgEnvironment::default(),
        prelude: injection.prelude,
    };
    let mut packages = injection.entries;
    packages.push(user);
    let primary = PackageId::from_usize(packages.len() - 1);
    let plan = seal_hir::PackagePlan { packages, primary };
    let mut loaded = injection.loaded;
    loaded.push((file_id, Arc::unwrap_or_clone(ast)));
    let mut dependencies = injection.parse_dependencies;
    dependencies.push(dependency);

    let Some(lowered) = session.profiler().time("hir", || {
        crate::frontend::lower_package(
            session,
            &input.display().to_string(),
            &plan,
            &loaded,
            dependencies,
        )
    }) else {
        return;
    };
    if session.diag().error_count() != errors {
        return;
    }
    let Lowered { hir, dependency } = lowered;
    let hir = hir
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    compile_hir(
        session,
        request,
        &hir,
        dependency,
        &input.display().to_string(),
        file_id,
        HirRequest {
            input,
            entry: None,
            profile: None,
            plan: None,
        },
    );
}

/// Compiles every input as one package with a module graph.
fn compile_package(session: &Session, request: Request<'_>, arguments: &CompileArgs) {
    let errors = session.diag().error_count();
    let mut loaded: Vec<(seal_span::FileId, seal_ast::Ast)> =
        Vec::with_capacity(arguments.inputs.len());
    let mut dependencies = Vec::with_capacity(arguments.inputs.len());
    for input in &arguments.inputs {
        let identity = input.display().to_string();
        let Ok(file_id) = session.load_file_persistent(input, &identity) else {
            return;
        };
        let Some(file) = session.source_map().file(file_id) else {
            return;
        };
        let Some(tokens) = session
            .profiler()
            .time("lex", || crate::frontend::lex(session, file_id, &file))
        else {
            return;
        };
        if session.diag().error_count() != errors {
            return;
        }
        let Some(ast) = session.profiler().time("parse", || {
            crate::frontend::parse(session, file_id, &file, &tokens)
        }) else {
            return;
        };
        if session.diag().error_count() != errors {
            return;
        }
        let dependency = match crate::frontend::ast_dependency(session, file_id, &file) {
            Ok(dependency) => dependency,
            Err(error) => {
                session
                    .diag()
                    .bug(format!("the parse dependency is missing: {error}"));
            }
        };
        dependencies.push(dependency);
        loaded.push((file_id, Arc::unwrap_or_clone(ast)));
    }
    let Some(&(primary_file, _)) = loaded.first() else {
        session
            .diag()
            .bug("a package plan needs at least one source file");
    };

    let name = session.interner().intern("main");
    let mut packages = seal_hir::PackagePlan::single(name, primary_file).packages;
    let entry = &mut packages[0];
    entry.files = loaded
        .iter()
        .enumerate()
        .map(|(index, (file, _))| seal_hir::PackagePlanFile {
            file: *file,
            primary: index == 0,
        })
        .collect();
    entry.exposed = expose_paths(session, &arguments.expose);

    let injection = match inject_single(session, stdlib_mode(arguments), arguments.no_prelude) {
        Ok(injection) => injection,
        Err(message) => {
            eprintln!("error: {message}");
            return;
        }
    };
    entry.dependencies = injection.package_dependencies;
    entry.prelude = injection.prelude;
    let mut all_packages = injection.entries;
    all_packages.append(&mut packages);
    let primary = PackageId::from_usize(all_packages.len() - 1);
    let plan = seal_hir::PackagePlan {
        packages: all_packages,
        primary,
    };
    let mut all_loaded = injection.loaded;
    all_loaded.append(&mut loaded);
    let mut all_dependencies = injection.parse_dependencies;
    all_dependencies.append(&mut dependencies);

    let unit_key = arguments
        .inputs
        .iter()
        .map(|input| input.display().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let Some(lowered) = session.profiler().time("hir", || {
        crate::frontend::lower_package(session, &unit_key, &plan, &all_loaded, all_dependencies)
    }) else {
        return;
    };
    if session.diag().error_count() != errors {
        return;
    }
    let Lowered { hir, dependency } = lowered;
    let hir = hir
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    compile_hir(
        session,
        request,
        &hir,
        dependency,
        &unit_key,
        primary_file,
        HirRequest {
            input: &arguments.inputs[0],
            entry: None,
            profile: None,
            plan: None,
        },
    );
}

/// The standard-library contribution to one single-package compilation.
struct Injection {
    entries: Vec<seal_hir::PackagePlanEntry>,
    loaded: Vec<(seal_span::FileId, seal_ast::Ast)>,
    parse_dependencies: Vec<seal_query::Dependency>,
    package_dependencies: Vec<seal_hir::PackagePlanDependency>,
    prelude: Option<seal_hir::PreludeSource>,
}

/// Selects the standard-library mode requested on the `sealc` command line.
fn stdlib_mode(arguments: &CompileArgs) -> seal_plan::PlanStdlibMode {
    if arguments.no_stdlib {
        seal_plan::PlanStdlibMode::None
    } else if arguments.core_only {
        seal_plan::PlanStdlibMode::Core
    } else {
        seal_plan::PlanStdlibMode::Std
    }
}

/// Loads the standard-library packages and prelude for one user package.
fn inject_single(
    session: &Session,
    mode: seal_plan::PlanStdlibMode,
    no_prelude: bool,
) -> Result<Injection, String> {
    if mode == seal_plan::PlanStdlibMode::None {
        return Ok(Injection {
            entries: Vec::new(),
            loaded: Vec::new(),
            parse_dependencies: Vec::new(),
            package_dependencies: Vec::new(),
            prelude: None,
        });
    }
    let stdlib = seal_stdlib::Stdlib::load().map_err(|error| error.to_string())?;
    let needed = crate::stdlib::needed_for_mode(mode);
    let injected = crate::stdlib::load(session, &stdlib, &needed, crate::stdlib::cfg(session), 1)?;
    let core = crate::stdlib::package_id(&injected, "core")?;
    let mut package_dependencies = vec![seal_hir::PackagePlanDependency {
        alias: session.interner().intern("core"),
        package: core,
    }];
    if mode == seal_plan::PlanStdlibMode::Std {
        let std = crate::stdlib::package_id(&injected, "std")?;
        package_dependencies.push(seal_hir::PackagePlanDependency {
            alias: session.interner().intern("std"),
            package: std,
        });
    }
    let prelude = if no_prelude {
        None
    } else {
        let package = match mode {
            seal_plan::PlanStdlibMode::Std => "std",
            seal_plan::PlanStdlibMode::Core => "core",
            seal_plan::PlanStdlibMode::None => unreachable!("handled above"),
        };
        let path = stdlib.prelude_for(package).ok_or_else(|| {
            format!("E2100: the standard library declares no prelude for `{package}`")
        })?;
        let (source, module) = path.split_once("::").ok_or_else(|| {
            format!("E2100: the standard-library prelude `{path}` is not a `package::module` path")
        })?;
        let module = module_path(session, module)
            .ok_or_else(|| format!("E2100: the standard-library prelude `{path}` is empty"))?;
        Some(seal_hir::PreludeSource {
            package: crate::stdlib::package_id(&injected, source)?,
            module,
        })
    };
    Ok(Injection {
        entries: injected.entries,
        loaded: injected.loaded,
        parse_dependencies: injected.dependencies,
        package_dependencies,
        prelude,
    })
}

/// Compiles one package graph from a versioned plan file.
fn compile_plan_file(
    session: &Session,
    request: Request<'_>,
    plan: CompilePlan,
    no_prelude: bool,
) -> bool {
    let errors = session.diag().error_count();
    let mut needed = std::collections::BTreeSet::new();
    for package in &plan.packages {
        needed.extend(crate::stdlib::needed_for_mode(package.stdlib));
        if let Some(prelude) = &package.prelude
            && let Some(name) = &prelude.stdlib
        {
            needed.insert(name.clone());
        }
    }
    let injected = if needed.is_empty() {
        None
    } else {
        let Some(info) = &plan.stdlib else {
            eprintln!(
                "error: E8141: the plan requires standard-library packages but names no standard-library identity"
            );
            return false;
        };
        let stdlib = match seal_stdlib::Stdlib::load() {
            Ok(stdlib) => stdlib,
            Err(error) => {
                eprintln!("error: {error}");
                return false;
            }
        };
        if stdlib.language() != info.language
            || stdlib.version() != info.version
            || stdlib.digest() != info.digest
        {
            eprintln!(
                "error: E8141: the plan requires standard library {} {}, digest {}, but the installed standard library is {} {}, digest {}",
                info.language,
                info.version,
                info.digest,
                stdlib.language(),
                stdlib.version(),
                stdlib.digest()
            );
            return false;
        }
        let cfg = stdlib_cfg(&plan.packages[plan.primary]);
        match crate::stdlib::load(session, &stdlib, &needed, cfg, plan.packages.len()) {
            Ok(injected) => Some(injected),
            Err(message) => {
                eprintln!("error: {message}");
                return false;
            }
        }
    };
    let (mut package_entries, mut loaded, mut dependencies, indexes) = match injected {
        Some(injected) => (
            injected.entries,
            injected.loaded,
            injected.dependencies,
            injected.indexes,
        ),
        None => (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            std::collections::BTreeMap::new(),
        ),
    };
    let injected_count = package_entries.len();
    let mut file_ids = std::collections::BTreeMap::new();
    let mut primary: Option<(seal_span::FileId, PathBuf)> = None;
    for (index, package) in plan.packages.iter().enumerate() {
        for (file_index, file) in package.files.iter().enumerate() {
            let identity = format!("{}#{index}#{file_index}", file.path.display());
            let Ok(file_id) = session.load_file_persistent(&file.path, &identity) else {
                return false;
            };
            let Some(source) = session.source_map().file(file_id) else {
                return false;
            };
            let Some(tokens) = session
                .profiler()
                .time("lex", || crate::frontend::lex(session, file_id, &source))
            else {
                return false;
            };
            if session.diag().error_count() != errors {
                return false;
            }
            let Some(ast) = session.profiler().time("parse", || {
                crate::frontend::parse(session, file_id, &source, &tokens)
            }) else {
                return false;
            };
            if session.diag().error_count() != errors {
                return false;
            }
            let dependency = match crate::frontend::ast_dependency(session, file_id, &source) {
                Ok(dependency) => dependency,
                Err(error) => {
                    session
                        .diag()
                        .bug(format!("the parse dependency is missing: {error}"));
                }
            };
            dependencies.push(dependency);
            if index == plan.primary && file.primary {
                primary = Some((file_id, file.path.clone()));
            }
            file_ids.insert((index, file.path.clone()), file_id);
            loaded.push((file_id, Arc::unwrap_or_clone(ast)));
        }
    }
    let Some((primary_file, primary_input)) = primary else {
        eprintln!("error: the compile plan has no primary source file");
        return false;
    };

    let mut package_plan = seal_hir::PackagePlan {
        packages: std::mem::take(&mut package_entries),
        primary: PackageId::from_usize(injected_count + plan.primary),
    };
    for (index, package) in plan.packages.iter().enumerate() {
        let name = session.interner().intern(&package.name);
        let mut files = Vec::with_capacity(package.files.len());
        for file in &package.files {
            let Some(id) = file_ids.get(&(index, file.path.clone())) else {
                eprintln!(
                    "error: the compile plan names the unloaded file `{}`",
                    file.path.display()
                );
                return false;
            };
            files.push(seal_hir::PackagePlanFile {
                file: *id,
                primary: file.primary,
            });
        }
        let mut exposed = Vec::with_capacity(package.exposed.len());
        for value in &package.exposed {
            let Some(module) = module_path(session, value) else {
                eprintln!(
                    "error: the package `{}` exposes the empty module path `{value}`",
                    package.name
                );
                return false;
            };
            exposed.push(module);
        }
        let mut aliases = std::collections::BTreeSet::new();
        let mut hir_dependencies = Vec::with_capacity(package.dependencies.len() + 2);
        for dependency in &package.dependencies {
            aliases.insert(dependency.alias.clone());
            hir_dependencies.push(seal_hir::PackagePlanDependency {
                alias: session.interner().intern(&dependency.alias),
                package: PackageId::from_usize(injected_count + dependency.package),
            });
        }
        for &(alias, name) in implicit_stdlib_dependencies(package.stdlib) {
            if !aliases.insert(alias.to_owned()) {
                eprintln!(
                    "error: E8136: the package `{}` already uses the reserved dependency alias `{alias}`",
                    package.name
                );
                return false;
            }
            let Some(id) = indexes.get(name) else {
                eprintln!(
                    "error: E8141: the plan selects the `{name}` standard library, which is not injected"
                );
                return false;
            };
            hir_dependencies.push(seal_hir::PackagePlanDependency {
                alias: session.interner().intern(alias),
                package: PackageId::from_usize(*id),
            });
        }
        let cfg = seal_hir::CfgEnvironment {
            features: package.cfg.features.iter().cloned().collect(),
            target_os: package.cfg.target_os.clone(),
            target_arch: package.cfg.target_arch.clone(),
            target_env: package.cfg.target_env.clone(),
            pointer_width: package.cfg.pointer_width,
            debug_assertions: package.cfg.debug_assertions,
            test: package.cfg.test,
            bench: package.cfg.bench,
            env: package
                .cfg
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        };
        let prelude = if no_prelude {
            None
        } else if let Some(source) = &package.prelude {
            let Some(module) = module_path(session, &source.module) else {
                eprintln!(
                    "error: the prelude of package `{}` has an empty module path",
                    package.name
                );
                return false;
            };
            let package_id = match (&source.package, &source.stdlib) {
                (Some(index), None) => PackageId::from_usize(index + injected_count),
                (None, Some(name)) => match indexes.get(name) {
                    Some(index) => PackageId::from_usize(*index),
                    None => {
                        eprintln!(
                            "error: E8141: the prelude of package `{}` names the standard-library package `{name}`, which is not injected",
                            package.name
                        );
                        return false;
                    }
                },
                _ => {
                    eprintln!(
                        "error: E8141: the prelude of package `{}` must name exactly one package",
                        package.name
                    );
                    return false;
                }
            };
            Some(seal_hir::PreludeSource {
                package: package_id,
                module,
            })
        } else {
            None
        };
        package_plan.packages.push(seal_hir::PackagePlanEntry {
            name,
            edition: package.edition(),
            instance: PackageInstanceId::from_usize(index),
            files,
            exposed,
            dependencies: hir_dependencies,
            cfg,
            prelude,
        });
    }
    let stdlib_key = plan
        .stdlib
        .as_ref()
        .map_or("none", |stdlib| stdlib.digest.as_str());
    let unit_key = format!(
        "plan:{}:{}:{stdlib_key}",
        plan.target,
        plan.packages
            .iter()
            .map(|package| format!("{}:{}", package.name, package.edition))
            .collect::<Vec<_>>()
            .join(",")
    );
    let Some(lowered) = session.profiler().time("hir", || {
        crate::frontend::lower_package(session, &unit_key, &package_plan, &loaded, dependencies)
    }) else {
        return false;
    };
    if session.diag().error_count() != errors {
        return false;
    }
    let Lowered { hir, dependency } = lowered;
    let hir = hir
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = match &plan.entry {
        Some(value) => match resolve_entry(session, &hir, value) {
            Some(definition) => Some(definition),
            None => return false,
        },
        None => None,
    };
    let request = request.with_planned_output(plan.output.clone());
    compile_hir(
        session,
        request,
        &hir,
        dependency,
        &unit_key,
        primary_file,
        HirRequest {
            input: &primary_input,
            entry,
            profile: Some(&plan.profile),
            plan: Some(&plan),
        },
    );
    true
}

/// Returns the implicit dependency aliases one mode adds to a user package.
fn implicit_stdlib_dependencies(
    mode: seal_plan::PlanStdlibMode,
) -> &'static [(&'static str, &'static str)] {
    match mode {
        seal_plan::PlanStdlibMode::None => &[],
        seal_plan::PlanStdlibMode::Core => &[("core", "core")],
        seal_plan::PlanStdlibMode::Std => &[("core", "core"), ("std", "std")],
    }
}

/// Derives the target configuration for a standard-library package.
fn stdlib_cfg(package: &seal_plan::PlanPackage) -> seal_hir::CfgEnvironment {
    seal_hir::CfgEnvironment {
        features: std::collections::BTreeSet::new(),
        target_os: package.cfg.target_os.clone(),
        target_arch: package.cfg.target_arch.clone(),
        target_env: package.cfg.target_env.clone(),
        pointer_width: package.cfg.pointer_width,
        debug_assertions: package.cfg.debug_assertions,
        test: false,
        bench: false,
        env: std::collections::BTreeMap::new(),
    }
}

pub(crate) fn module_path(session: &Session, value: &str) -> Option<seal_hir::ModulePath> {
    if value == "root" {
        return Some(seal_hir::ModulePath::root());
    }
    let segments: Vec<_> = value
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(|segment| session.interner().intern(segment))
        .collect();
    (!segments.is_empty()).then(|| seal_hir::ModulePath::new(segments))
}

/// Parses the `--expose` values into module paths.
fn expose_paths(session: &Session, values: &[String]) -> Vec<seal_hir::ModulePath> {
    if values.is_empty() {
        return vec![seal_hir::ModulePath::root()];
    }
    let mut paths = Vec::with_capacity(values.len());
    for value in values {
        if value == "root" {
            paths.push(seal_hir::ModulePath::root());
            continue;
        }
        let segments: Vec<seal_symbol::Symbol> = value
            .split("::")
            .filter(|segment| !segment.is_empty())
            .map(|segment| session.interner().intern(segment))
            .collect();
        if segments.is_empty() {
            session
                .diag()
                .error(format!("the exposed module path `{value}` is empty"))
                .code(seal_error_codes::E2081.diag_code())
                .help("write a path such as `a::b`, or `root`")
                .emit();
            continue;
        }
        paths.push(seal_hir::ModulePath::new(segments));
    }
    paths
}

/// Runs every phase after lowering for one complete program.
fn compile_hir(
    session: &Session,
    request: Request<'_>,
    hir: &seal_hir::Hir,
    hir_dependency: seal_query::Dependency,
    unit_key: &str,
    file_id: seal_span::FileId,
    planned: HirRequest<'_>,
) {
    let HirRequest {
        input,
        entry,
        profile,
        plan,
    } = planned;
    let errors = session.diag().error_count();

    // Declarations and every body are separate queries. The final table is the
    // merge of the declarations pass and each body-local result, preserving the
    // same lookup semantics as a whole-file check.
    let Some(declarations) = session.profiler().time("typeck", || {
        body_cache::declarations(session, unit_key, hir, hir_dependency.clone())
    }) else {
        return;
    };
    if session.diag().error_count() != errors {
        return;
    }
    let body_unit = body_cache::BodyUnit {
        session,
        unit_key,
        hir,
        hir_dependency: hir_dependency.clone(),
        declarations: &declarations,
    };
    let mut typeck = declarations.results.as_ref().clone();
    let mut body_dependencies = Vec::with_capacity(hir.bodies().len());
    for (index, body) in hir.bodies().iter().enumerate() {
        let body_id = seal_hir::BodyId::from_usize(index);
        if body.kind == seal_hir::BodyKind::Closure {
            body_dependencies.push(None);
            continue;
        }
        let Some((checked, dependency)) = session.profiler().time("typeck-body", || {
            body_cache::typeck_body(&body_unit, body_id)
        }) else {
            return;
        };
        if session.diag().error_count() != errors {
            return;
        }
        typeck.merge(checked);
        body_dependencies.push(Some(dependency));
    }

    let mut mir_dependencies = Vec::with_capacity(hir.bodies().len());
    let mut mir = session.profiler().time("mir", || {
        let mut mir = seal_mir::Mir::new();
        for (index, body) in hir.bodies().iter().enumerate() {
            let body_id = seal_hir::BodyId::from_usize(index);
            match &body_dependencies[index] {
                Some(dependency) if body.kind != seal_hir::BodyKind::Closure => {
                    let Some((lowered, mir_dependency)) =
                        body_cache::mir_body(&body_unit, body_id, &typeck, dependency.clone())
                    else {
                        return mir;
                    };
                    mir_dependencies.push(Some(mir_dependency));
                    mir.push_body(lowered);
                }
                _ => {
                    mir_dependencies.push(None);
                    mir.push_body(seal_mir::lower_body(hir, &typeck, body));
                }
            }
        }
        mir
    });
    if session.diag().error_count() != errors {
        return;
    }
    if let Err(error) = mir.validate_with(hir) {
        session
            .diag()
            .bug(format!("MIR lowering produced an invalid body: {error}"));
    }
    // Drop elaboration runs before constant evaluation: it removes the sites
    // whose value is already gone, so a constant is not rejected for a
    // destructor it never runs. Borrow checking then sees the guards and the
    // replacement drops the program actually executes.
    let elaborated = session.profiler().time("elaborate-drops", || {
        seal_mir::elaborate_drops(hir, &mut mir)
    });
    if let Err(error) = elaborated {
        session
            .diag()
            .bug(format!("drop elaboration failed: {error}"));
    }
    if let Err(error) = mir.validate_with(hir) {
        session.diag().bug(format!(
            "drop elaboration produced an invalid body: {error}"
        ));
    }
    // Constant evaluation runs before borrow checking: a constant that cannot
    // be evaluated is an error in the initializer, not an ownership question.
    // Later phases read the resulting program instead of evaluating again.
    let program = session.profiler().time("const-eval", || {
        seal_const_eval::evaluate(
            hir,
            &mir,
            session.diag(),
            session.interner(),
            session.target().pointer_width(),
        )
    });
    if session.diag().error_count() != errors {
        return;
    }
    let aliasing = session.profiler().time("borrowck", || {
        seal_borrowck::check(
            hir,
            &mir,
            Some(&program),
            session.diag(),
            session.interner(),
        )
    });
    if session.diag().error_count() != errors {
        return;
    }

    // Collection runs even for `check`: it resolves symbolic trait calls and
    // proves the program has a finite, fully concrete instance set.
    let mono_context = crate::mono_cache::MonoContext {
        session,
        unit_key,
        hir,
        hir_dependency: hir_dependency.clone(),
        declarations_dependency: declarations.dependency.clone(),
    };
    let mono = session.profiler().time("mono", || {
        seal_mono::collect_with_specializer(
            hir,
            &mir,
            &program,
            session.diag(),
            session.interner(),
            |instance, owner, body| {
                let mir_dependency = hir
                    .body_id_of(owner)
                    .and_then(|id| mir_dependencies[id.index() as usize].clone());
                crate::mono_cache::specialize(&mono_context, mir_dependency, instance, owner, body)
            },
        )
    });
    if session.diag().error_count() != errors {
        return;
    }
    if let Err(error) = mono.validate(hir) {
        session.diag().bug(format!(
            "monomorphization produced an invalid instance: {error}"
        ));
    }
    if let Err(error) = mono.validate_globals(&program) {
        session.diag().bug(format!(
            "monomorphization produced an invalid global item: {error}"
        ));
    }

    let Request::Build {
        emit,
        output,
        runtime_archive,
        link,
        planned_output,
        link_metadata,
        cgu_count,
    } = &request
    else {
        if let Request::Api { path } = &request {
            let package_version = plan
                .and_then(|plan| plan.packages.get(plan.primary))
                .map(|package| package.version.as_str())
                .unwrap_or("0.0.0");
            if let Err(error) = api::write(session, hir, package_version, path) {
                session.diag().bug(error);
            }
        }
        return;
    };
    let output_path = planned_output
        .clone()
        .unwrap_or_else(|| artifact_path(output, input, *emit));
    if let Some(parent) = output_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            session.diag().bug(format!(
                "cannot create the artifact directory `{}`: {error}",
                parent.display()
            ));
        }
    }
    let opt_level = profile.map_or(output.optimization.opt_level, |profile| {
        match profile.opt_level {
            0 => seal_session::OptLevel::O0,
            1 => seal_session::OptLevel::O1,
            2 => seal_session::OptLevel::O2,
            _ => seal_session::OptLevel::O3,
        }
    });
    let target_spec = plan
        .and_then(|plan| plan.target_spec.as_ref())
        .and_then(|target| target.to_spec().ok());
    let mut link_args = link.link_args.clone();
    if let Some(target) = plan.and_then(|plan| plan.target_spec.as_ref()) {
        link_args.extend(target.link_args.iter().map(OsString::from));
    }
    let options = CodegenOptions {
        emit: *emit,
        output: output_path,
        runtime: target_spec
            .as_ref()
            .map_or(seal_target::RuntimePolicy::Default, |spec| spec.runtime()),
        panic: target_spec
            .as_ref()
            .map_or(seal_target::PanicStrategy::Abort, |spec| spec.panic()),
        linker: target_spec
            .as_ref()
            .and_then(|spec| spec.linker().map(ToOwned::to_owned)),
        linker_script: target_spec
            .as_ref()
            .and_then(|spec| spec.linker_script().map(Path::to_path_buf)),
        sysroot: target_spec
            .as_ref()
            .and_then(|spec| spec.sysroot().map(Path::to_path_buf)),
        source_date_epoch: plan
            .and_then(|plan| plan.target_spec.as_ref())
            .map_or(0, |target| target.source_date_epoch),
        reproducible: plan
            .and_then(|plan| plan.target_spec.as_ref())
            .is_none_or(|target| target.reproducible),
        target_triple: plan.map(|plan| plan.target.clone()),
        opt_level,
        debug_info: profile.map_or(output.optimization.debug_info, |profile| profile.debug),
        crate_name: input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned()),
        runtime_archive: runtime_archive.map(Path::to_path_buf),
        libraries: link.libraries.clone(),
        search_paths: link.search_paths.clone(),
        link_args,
        entry,
        harness: match plan.map(|plan| plan.metadata.kind.as_str()) {
            Some("test") => Some(seal_codegen::HarnessKind::Test),
            Some("bench") => Some(seal_codegen::HarnessKind::Bench),
            _ => None,
        },
        cgu_count: cgu_count
            .as_ref()
            .copied()
            .unwrap_or_else(|| profile.map_or(1, |profile| profile.codegen_units)),
    };
    let codegen_input = CodegenInput {
        mono: &mono,
        constants: &program,
        hir,
        source_map: session.source_map(),
        interner: session.interner(),
        target: session.target(),
        file: file_id,
        noalias_functions: aliasing.noalias_functions(),
    };
    let result = session
        .profiler()
        .time("codegen", || seal_codegen::emit(codegen_input, &options));
    if let Err(error) = result {
        report::codegen_error(session, &error);
        return;
    }
    if let Some(path) = link_metadata
        && let Err(error) = write_link_metadata(session, hir, path)
    {
        session.diag().bug(error);
    }
}

/// Writes the libraries named by `link` directives for a later link action.
#[derive(serde::Serialize)]
struct LinkMetadata {
    version: u32,
    libraries: Vec<String>,
}

fn write_link_metadata(session: &Session, hir: &seal_hir::Hir, path: &Path) -> Result<(), String> {
    let metadata = LinkMetadata {
        version: 1,
        libraries: hir
            .link_directives()
            .iter()
            .map(|directive| session.interner().resolve(directive.name).to_owned())
            .collect(),
    };
    let text = serde_json::to_string_pretty(&metadata)
        .map_err(|error| format!("cannot serialize link metadata: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create the link metadata directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("cannot replace `{}`: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use seal_query::QueryKind;

    use super::*;
    use crate::cli::Cli;

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, source: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("seal-incremental-{}-{name}.sl", std::process::id()));
            std::fs::write(&path, source).expect("the test source is writable");
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn check(path: &Path, tag: &str) -> (crate::cli::CompileArgs, Outcome) {
        let cli = Cli::parse_from([
            "sealc".to_owned(),
            "check".to_owned(),
            path.display().to_string(),
            "--remap-path-prefix".to_owned(),
            format!("/seal-test-{tag}-never-present={tag}"),
        ])
        .expect("valid check arguments");
        let crate::cli::CliCommand::Check(args) = cli.command else {
            unreachable!("the command is check");
        };
        let outcome = compile(&args.compile, Request::Check).expect("the check runs");
        (args.compile, outcome)
    }

    #[test]
    fn editing_one_body_reuses_the_other_bodies() {
        let file = TempFile::new(
            "body-edit",
            "func first() -> i32 { 1 }\nfunc second() -> i32 { 2 }\n",
        );
        let (arguments, outcome) = check(&file.0, "body");
        assert!(matches!(outcome, Outcome::Success));
        let options = session_options(&arguments, None);
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        session.queries().reset_stats();
        drop(session);

        std::fs::write(
            &file.0,
            "func first() -> i32 { 1 }\nfunc second() -> i32 { 3 }\n",
        )
        .expect("the edit is writable");
        let (_, outcome) = check(&file.0, "body");
        assert!(matches!(outcome, Outcome::Success));
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after = session.queries().stats();
        let misses = |kind| after.by_kind.get(&kind).map_or(0, |stats| stats.misses);
        let hits = |kind| after.by_kind.get(&kind).map_or(0, |stats| stats.hits);

        assert_eq!(misses(QueryKind::Tokens), 1);
        assert_eq!(misses(QueryKind::Ast), 1);
        assert_eq!(misses(QueryKind::ModuleItems), 1);
        assert_eq!(misses(QueryKind::Signature), 0);
        assert_eq!(
            misses(QueryKind::TypeckBody),
            1,
            "only the edited body is checked"
        );
        assert_eq!(
            misses(QueryKind::MirBody),
            1,
            "only the edited body is lowered"
        );
        assert_eq!(misses(QueryKind::MonoInstance), 1);
        assert!(hits(QueryKind::TypeckBody) >= 1);
        assert!(hits(QueryKind::MirBody) >= 1);
        assert!(hits(QueryKind::MonoInstance) >= 1);
    }

    #[test]
    fn cached_body_diagnostics_are_replayed() {
        let file = TempFile::new(
            "diagnostic-replay",
            "func first() -> i32 { 1 }\nfunc second() -> i32 { true }\n",
        );
        let (arguments, outcome) = check(&file.0, "replay");
        assert!(matches!(outcome, Outcome::Failed(_)));
        let options = session_options(&arguments, None);
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(session.diag().error_count(), 1);
        drop(session);

        let (_, outcome) = check(&file.0, "replay");
        assert!(matches!(outcome, Outcome::Failed(_)));
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(
            session.diag().error_count(),
            1,
            "a cache hit must replay the body's diagnostic"
        );
    }

    #[test]
    fn editing_a_signature_invalidates_bodies_without_rechecking_the_file() {
        let file = TempFile::new(
            "signature-edit",
            "func first(value: i32) -> i32 { value }\nfunc second() -> i32 { 2 }\n",
        );
        let (arguments, outcome) = check(&file.0, "signature");
        assert!(matches!(outcome, Outcome::Success));
        let options = session_options(&arguments, None);
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        session.queries().reset_stats();
        drop(session);

        std::fs::write(
            &file.0,
            "func first(value: i64) -> i64 { value }\nfunc second() -> i32 { 2 }\n",
        )
        .expect("the edit is writable");
        let (_, outcome) = check(&file.0, "signature");
        assert!(matches!(outcome, Outcome::Success));
        let session = crate::session_cache::acquire(&options).expect("the session exists");
        let session = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after = session.queries().stats();
        assert_eq!(after.by_kind[&QueryKind::Signature].misses, 1);
        assert_eq!(after.by_kind[&QueryKind::Tokens].misses, 1);
        assert!(after.by_kind[&QueryKind::TypeckBody].misses >= 2);
    }
}
