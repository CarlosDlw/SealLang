//! `sealc`, the Seal compiler driver.
//!
//! The driver owns process-level concerns only: argument parsing, exit codes,
//! panic handling, and translating a [`seal_session::Session`] into a result. It
//! performs no language analysis itself.

mod args;

use std::panic::AssertUnwindSafe;
use std::process::ExitCode;

use seal_diagnostics::IcePanic;
use seal_session::{Options, Session};

fn main() -> ExitCode {
    install_panic_hook();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match std::panic::catch_unwind(AssertUnwindSafe(|| run(&argv))) {
        Ok(Ok(code)) => code,
        Ok(Err(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
        Err(_) => ExitCode::from(101),
    }
}

/// Suppresses the default panic report for internal compiler errors; the
/// diagnostic emitter already printed a structured report.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().is::<IcePanic>() {
            return;
        }
        previous(info);
    }));
}

fn run(argv: &[String]) -> Result<ExitCode, String> {
    let args = args::parse(argv.iter().cloned()).map_err(|error| error.to_string())?;

    if args.show_help {
        print!("{}", args::usage());
        return Ok(ExitCode::SUCCESS);
    }
    if args.show_version {
        println!("sealc {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(code) = &args.explain {
        return match seal_error_codes::explain(code) {
            Some(explanation) => {
                print!("{explanation}");
                Ok(ExitCode::SUCCESS)
            }
            None => Err(format!("unknown error code `{code}`")),
        };
    }

    let target = match &args.target {
        Some(triple) => {
            seal_target::TargetTriple::parse(triple).map_err(|error| error.to_string())?
        }
        None => seal_target::host_triple(),
    };
    let language_version = match &args.language_version {
        Some(version) => {
            seal_version::LanguageVersion::parse(version).map_err(|error| error.to_string())?
        }
        None => seal_version::LanguageVersion::CURRENT,
    };
    let options = Options {
        target,
        opt_level: args.opt_level,
        error_format: args.error_format,
        color: args.color,
        language_version,
        allow_unstable: args.allow_unstable,
        remap_prefixes: args.remap.clone(),
        time_passes: args.time_passes,
        max_errors: args
            .max_errors
            .unwrap_or(seal_diagnostics::DEFAULT_ERROR_LIMIT),
        ..Options::default()
    };

    let session = Session::new(options).map_err(|error| error.to_string())?;

    for input in &args.inputs {
        let Ok(file_id) = session.load_file(input) else {
            continue;
        };
        let Some(file) = session.source_map().file(file_id) else {
            continue;
        };
        let errors_before_lexing = session.diag().error_count();
        let lexed = session.profiler().time("lex", || {
            seal_lexer::lex(&file, session.diag(), session.interner())
        });
        // A lexical error leaves `Error` tokens behind, and parsing them only
        // adds secondary syntax errors; the parser runs on a clean token stream.
        if session.diag().error_count() == errors_before_lexing {
            let ast = session.profiler().time("parse", || {
                seal_parser::parse(&lexed, session.diag(), session.interner())
            });
            // Lowering assumes a syntactically valid tree. A parse error already
            // leaves `Error` nodes behind, so the pipeline stops after `parse`.
            if session.diag().error_count() == errors_before_lexing {
                let hir = session.profiler().time("hir", || {
                    seal_hir::lower(&ast, session.diag(), session.interner())
                });
                if session.diag().error_count() == errors_before_lexing {
                    let typeck = session.profiler().time("typeck", || {
                        seal_typeck::check(
                            &hir,
                            session.diag(),
                            session.interner(),
                            session.target().pointer_width(),
                        )
                    });
                    // Lowering relies on the invariants the checker establishes,
                    // so it runs only when no error was reported.
                    if session.diag().error_count() == errors_before_lexing {
                        let mir = session
                            .profiler()
                            .time("mir", || seal_mir::lower(&hir, &typeck));
                        if let Err(error) = mir.validate() {
                            session
                                .diag()
                                .bug(format!("MIR lowering produced an invalid body: {error}"));
                        }
                    }
                }
            }
        }
    }

    if let Some(error) = session.diag().emitter_error() {
        return Err(format!("failed to write diagnostics: {error}"));
    }
    if args.time_passes {
        eprint!("{}", session.profiler().report());
    }

    if session.abort_if_errors().is_err() {
        let errors = session.diag().error_count();
        eprintln!(
            "error: could not compile {} input file(s) due to {errors} previous error(s)",
            args.inputs.len()
        );
        return Ok(ExitCode::from(1));
    }

    // Later phases (move analysis, borrow checking, codegen) plug in here,
    // consuming the MIR produced above.
    Ok(ExitCode::SUCCESS)
}
