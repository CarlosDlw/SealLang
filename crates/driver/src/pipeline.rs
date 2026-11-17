//! Driving every compiler phase for each requested input.
//!
//! Each phase runs only when the previous one reported no errors: later phases
//! assume the invariants their predecessors establish. The control flow is
//! written as early returns so the order of those checks stays obvious.
//!
//! The request type makes the difference between building and checking
//! structural: only a build carries an artifact to write, so no command can
//! accidentally produce files or silently skip code generation.

use std::path::Path;
use std::process::ExitCode;

use seal_codegen::{CodegenInput, CodegenOptions, Emit};
use seal_session::Session;

use crate::cli::{BuildArgs, CheckArgs, CompileArgs, OutputArgs};
use crate::options::session_options;
use crate::output::artifact_path;
use crate::report;

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
}

/// Analyzes every input without writing anything.
pub(crate) fn check(args: &CheckArgs) -> Result<ExitCode, String> {
    compile(&args.compile, Request::Check)
}

/// Creates the session, compiles each input, and reports the final status.
fn compile(arguments: &CompileArgs, request: Request<'_>) -> Result<ExitCode, String> {
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
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
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

    let mir = session
        .profiler()
        .time("mir", || seal_mir::lower(&hir, &typeck));
    if let Err(error) = mir.validate_with(&hir) {
        session
            .diag()
            .bug(format!("MIR lowering produced an invalid body: {error}"));
    }
    let aliasing = session.profiler().time("borrowck", || {
        seal_borrowck::check(&hir, &mir, session.diag(), session.interner())
    });
    if session.diag().error_count() != errors {
        return;
    }

    // Collection runs even for `check`: it resolves symbolic trait calls and
    // proves the program has a finite, fully concrete instance set.
    let mono = session.profiler().time("mono", || {
        seal_mono::collect(&hir, &mir, session.diag(), session.interner())
    });
    if session.diag().error_count() != errors {
        return;
    }
    if let Err(error) = mono.validate(&hir) {
        session.diag().bug(format!(
            "monomorphization produced an invalid instance: {error}"
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
        opt_level: output.opt_level,
        debug_info: output.debug_info,
        crate_name: input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned()),
        runtime_archive: runtime_archive.map(Path::to_path_buf),
    };
    let codegen_input = CodegenInput {
        mono: &mono,
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
