//! Driving every compiler phase for each requested input.
//!
//! Each phase runs only when the previous one reported no errors: later phases
//! assume the invariants their predecessors establish. The control flow is
//! written as early returns so the order of those checks stays obvious.

use std::path::Path;
use std::process::ExitCode;

use seal_codegen::{CodegenInput, CodegenOptions};
use seal_session::Session;

use crate::cli::{Cli, RequestedOutput};
use crate::options::session_options;
use crate::output::artifact_path;
use crate::report;

/// Compiles every input and returns the process exit status.
pub(crate) fn compile(cli: &Cli) -> Result<ExitCode, String> {
    let session = Session::new(session_options(cli)).map_err(|error| error.to_string())?;
    let requested = cli.output.requested();
    for input in &cli.inputs {
        compile_input(&session, cli, requested, input);
    }

    if let Some(error) = session.diag().emitter_error() {
        return Err(format!("failed to write diagnostics: {error}"));
    }
    if cli.diagnostics.time_passes {
        eprint!("{}", session.profiler().report());
    }
    if session.abort_if_errors().is_err() {
        let errors = session.diag().error_count();
        eprintln!(
            "error: could not compile {} input file(s) due to {errors} previous error(s)",
            cli.inputs.len()
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs the whole pipeline for one input file.
fn compile_input(session: &Session, cli: &Cli, requested: RequestedOutput, input: &Path) {
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
    if let Err(error) = mir.validate() {
        session
            .diag()
            .bug(format!("MIR lowering produced an invalid body: {error}"));
    }
    session.profiler().time("borrowck", || {
        seal_borrowck::check(&hir, &mir, session.diag(), session.interner())
    });
    if session.diag().error_count() != errors {
        return;
    }

    let Some(emit) = requested.artifact() else {
        return;
    };
    let options = CodegenOptions {
        emit,
        output: artifact_path(cli, input, emit),
        opt_level: cli.output.opt_level,
        debug_info: cli.output.debug_info,
        crate_name: input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned()),
        runtime_archive: cli.runtime.runtime_archive.clone(),
    };
    let codegen_input = CodegenInput {
        mir: &mir,
        hir: &hir,
        source_map: session.source_map(),
        interner: session.interner(),
        target: session.target(),
        file: file_id,
    };
    let result = session
        .profiler()
        .time("codegen", || seal_codegen::emit(codegen_input, &options));
    if let Err(error) = result {
        report::codegen_error(session, &error);
    }
}
