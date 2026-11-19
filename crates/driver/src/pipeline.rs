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
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus};

use seal_codegen::{CodegenInput, CodegenOptions, Emit};
use seal_session::Session;
use seal_target::host_triple;

use crate::cli::{BuildArgs, CheckArgs, CompileArgs, EmitArg, OutputArgs, RunArgs};
use crate::options::session_options;
use crate::output::artifact_path;
use crate::report;
use crate::temp::TemporaryDirectory;

/// What the selected command asks the pipeline to do.
#[derive(Clone, Copy)]
enum Request<'a> {
    /// Run analysis through monomorphization and write nothing.
    Check,
    /// Run the whole pipeline and write an artifact.
    Build {
        emit: Emit,
        output: &'a OutputArgs,
        runtime_archive: Option<&'a Path>,
    },
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
    /// Returns the output options that affect session configuration, if any.
    fn output(&self) -> Option<&OutputArgs> {
        match self {
            Self::Check => None,
            Self::Build { output, .. } => Some(output),
        }
    }

    /// Returns the verb that describes this request in the failure summary.
    const fn action(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build { .. } => "compile",
        }
    }
}

/// Compiles every input and writes the requested artifacts.
pub(crate) fn build(args: &BuildArgs) -> Result<ExitCode, String> {
    compile(
        &args.compile,
        Request::Build {
            emit: args.output.artifact(),
            output: &args.output,
            runtime_archive: args.runtime_archive.as_deref(),
        },
    )
    .map(Outcome::code)
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
    };
    let request = Request::Build {
        emit: Emit::Executable,
        output: &output,
        runtime_archive: args.runtime_archive.as_deref(),
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

/// Analyzes every input without writing anything.
pub(crate) fn check(args: &CheckArgs) -> Result<ExitCode, String> {
    compile(&args.compile, Request::Check).map(Outcome::code)
}

/// Creates the session, compiles each input, and reports the final status.
fn compile(arguments: &CompileArgs, request: Request<'_>) -> Result<Outcome, String> {
    let session = Session::new(session_options(arguments, request.output()))
        .map_err(|error| error.to_string())?;
    for input in &arguments.inputs {
        compile_input(&session, request, input);
    }

    if let Some(error) = session.diag().emitter_error() {
        return Err(format!("failed to write diagnostics: {error}"));
    }
    if arguments.diagnostics.time_passes {
        eprint!("{}", session.profiler().report());
    }
    if session.abort_if_errors().is_err() {
        let files = arguments.inputs.len();
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
        seal_hir::lower(&ast, session.diag(), session.interner())
    });
    if session.diag().error_count() != errors {
        return;
    }

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
    } = request
    else {
        return;
    };
    let options = CodegenOptions {
        emit,
        output: artifact_path(output, input, emit),
        opt_level: output.optimization.opt_level,
        debug_info: output.optimization.debug_info,
        crate_name: input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned()),
        runtime_archive: runtime_archive.map(Path::to_path_buf),
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
    }
}
