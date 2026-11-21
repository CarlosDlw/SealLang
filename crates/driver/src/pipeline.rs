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

use seal_codegen::{CodegenInput, CodegenOptions, Emit};
use seal_def::{DefId, PackageId, PackageInstanceId};
use seal_index::Idx;
use seal_metadata::read_archive_file;
use seal_plan::CompilePlan;
use seal_session::Session;
use seal_target::host_triple;

use crate::api;
use crate::cli::plan::resolve_entry;

use crate::cli::{BuildArgs, CheckArgs, CompileArgs, EmitArg, LinkArgs, OutputArgs, RunArgs};
use crate::options::session_options;
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
    let session = Session::new(options).map_err(|error| error.to_string())?;
    if let Some(plan) = plan {
        if !compile_plan_file(&session, request.clone(), plan) {
            return Ok(Outcome::Failed(ExitCode::from(1)));
        }
    } else if arguments.plan {
        compile_package(&session, request.clone(), arguments);
    } else {
        for input in &arguments.inputs {
            compile_input(&session, request.clone(), input);
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
    Ok(Outcome::Success)
}

/// Selects the singular or plural spelling of a counted noun.
const fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

/// Runs the whole pipeline for one input file.
fn compile_input(session: &Session, request: Request<'_>, input: &Path) {
    let Ok(file_id) = session.load_file(input) else {
        return;
    };
    let Some(file) = session.source_map().file(file_id) else {
        return;
    };
    let errors = session.diag().error_count();

    let tokens = session.profiler().time("lex", || {
        seal_lexer::lex(&file, session.diag(), session.interner())
    });
    // A lexical error leaves `Error` tokens behind, and parsing them only adds
    // secondary syntax errors; the parser runs on a clean token stream.
    if session.diag().error_count() != errors {
        return;
    }

    let ast = session.profiler().time("parse", || {
        seal_parser::parse(&tokens, session.diag(), session.interner())
    });
    // Lowering assumes a syntactically valid tree. A parse error already leaves
    // `Error` nodes behind, so the pipeline stops after `parse`.
    if session.diag().error_count() != errors {
        return;
    }

    let hir = session.profiler().time("hir", || {
        seal_hir::lower_with_c_abi(
            &ast,
            session.diag(),
            session.interner(),
            session.target().c_abi(),
        )
    });
    if session.diag().error_count() != errors {
        return;
    }

    compile_hir(
        session,
        request,
        hir,
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
    for input in &arguments.inputs {
        let Ok(file_id) = session.load_file(input) else {
            return;
        };
        let Some(file) = session.source_map().file(file_id) else {
            return;
        };
        let tokens = session.profiler().time("lex", || {
            seal_lexer::lex(&file, session.diag(), session.interner())
        });
        if session.diag().error_count() != errors {
            return;
        }
        let ast = session.profiler().time("parse", || {
            seal_parser::parse(&tokens, session.diag(), session.interner())
        });
        if session.diag().error_count() != errors {
            return;
        }
        loaded.push((file_id, ast));
    }
    let Some(&(primary_file, _)) = loaded.first() else {
        session
            .diag()
            .bug("a package plan needs at least one source file");
    };

    let name = session.interner().intern("main");
    let mut plan = seal_hir::PackagePlan::single(name, primary_file);
    let entry = &mut plan.packages[0];
    entry.files = loaded
        .iter()
        .enumerate()
        .map(|(index, (file, _))| seal_hir::PackagePlanFile {
            file: *file,
            primary: index == 0,
        })
        .collect();
    entry.exposed = expose_paths(session, &arguments.expose);

    let hir = session.profiler().time("hir", || {
        seal_hir::lower_package(
            &plan,
            &loaded,
            session.diag(),
            session.interner(),
            session.target().c_abi(),
        )
    });
    if session.diag().error_count() != errors {
        return;
    }
    compile_hir(
        session,
        request,
        hir,
        primary_file,
        HirRequest {
            input: &arguments.inputs[0],
            entry: None,
            profile: None,
            plan: None,
        },
    );
}

/// Compiles one package graph from a versioned plan file.
fn compile_plan_file(session: &Session, request: Request<'_>, plan: CompilePlan) -> bool {
    let mut loaded: Vec<(seal_span::FileId, seal_ast::Ast)> = Vec::new();
    let mut file_ids = std::collections::BTreeMap::new();
    let mut primary: Option<(seal_span::FileId, PathBuf)> = None;
    let errors = session.diag().error_count();
    for (index, package) in plan.packages.iter().enumerate() {
        for file in &package.files {
            let Ok(file_id) = session.load_file_distinct(&file.path) else {
                return false;
            };
            let Some(source) = session.source_map().file(file_id) else {
                return false;
            };
            let tokens = session.profiler().time("lex", || {
                seal_lexer::lex(&source, session.diag(), session.interner())
            });
            if session.diag().error_count() != errors {
                return false;
            }
            let ast = session.profiler().time("parse", || {
                seal_parser::parse(&tokens, session.diag(), session.interner())
            });
            if session.diag().error_count() != errors {
                return false;
            }
            if index == plan.primary && file.primary {
                primary = Some((file_id, file.path.clone()));
            }
            file_ids.insert((index, file.path.clone()), file_id);
            loaded.push((file_id, ast));
        }
    }
    let Some((primary_file, primary_input)) = primary else {
        eprintln!("error: the compile plan has no primary source file");
        return false;
    };

    let mut package_plan = seal_hir::PackagePlan {
        packages: Vec::with_capacity(plan.packages.len()),
        primary: PackageId::from_usize(plan.primary),
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
        let exposed = package
            .exposed
            .iter()
            .filter_map(|value| module_path(session, value))
            .collect();
        let dependencies = package
            .dependencies
            .iter()
            .map(|dependency| seal_hir::PackagePlanDependency {
                alias: session.interner().intern(&dependency.alias),
                package: PackageId::from_usize(dependency.package),
            })
            .collect();
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
        package_plan.packages.push(seal_hir::PackagePlanEntry {
            name,
            instance: PackageInstanceId::from_usize(index),
            files,
            exposed,
            dependencies,
            cfg,
        });
    }
    let hir = session.profiler().time("hir", || {
        seal_hir::lower_package(
            &package_plan,
            &loaded,
            session.diag(),
            session.interner(),
            session.target().c_abi(),
        )
    });
    if session.diag().error_count() != errors {
        return false;
    }
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
        hir,
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

fn module_path(session: &Session, value: &str) -> Option<seal_hir::ModulePath> {
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
    hir: seal_hir::Hir,
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

    // Lowering relies on the invariants the checker establishes, so it runs
    // only when no error was reported.
    let typeck = session.profiler().time("typeck", || {
        seal_typeck::check(
            &hir,
            session.diag(),
            session.interner(),
            session.target().pointer_width(),
        )
    });
    if session.diag().error_count() != errors {
        return;
    }

    let mut mir = session
        .profiler()
        .time("mir", || seal_mir::lower(&hir, &typeck));
    if let Err(error) = mir.validate_with(&hir) {
        session
            .diag()
            .bug(format!("MIR lowering produced an invalid body: {error}"));
    }
    // Drop elaboration runs before constant evaluation: it removes the sites
    // whose value is already gone, so a constant is not rejected for a
    // destructor it never runs. Borrow checking then sees the guards and the
    // replacement drops the program actually executes.
    let elaborated = session.profiler().time("elaborate-drops", || {
        seal_mir::elaborate_drops(&hir, &mut mir)
    });
    if let Err(error) = elaborated {
        session
            .diag()
            .bug(format!("drop elaboration failed: {error}"));
    }
    if let Err(error) = mir.validate_with(&hir) {
        session.diag().bug(format!(
            "drop elaboration produced an invalid body: {error}"
        ));
    }
    // Constant evaluation runs before borrow checking: a constant that cannot
    // be evaluated is an error in the initializer, not an ownership question.
    // Later phases read the resulting program instead of evaluating again.
    let program = session.profiler().time("const-eval", || {
        seal_const_eval::evaluate(
            &hir,
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
            &hir,
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
    let mono = session.profiler().time("mono", || {
        seal_mono::collect(&hir, &mir, &program, session.diag(), session.interner())
    });
    if session.diag().error_count() != errors {
        return;
    }
    if let Err(error) = mono.validate(&hir) {
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
            if let Err(error) = api::write(session, &hir, package_version, path) {
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
        cgu_count: cgu_count
            .as_ref()
            .copied()
            .unwrap_or_else(|| profile.map_or(1, |profile| profile.codegen_units)),
    };
    let codegen_input = CodegenInput {
        mono: &mono,
        constants: &program,
        hir: &hir,
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
        && let Err(error) = write_link_metadata(session, &hir, path)
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
