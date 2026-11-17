//! Tests for the `run` command surface.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::CommandFactory;
use clap::error::ErrorKind;
use seal_session::OptLevel;

use crate::cli::Cli;

use super::{parse, run_args};

#[test]
fn run_defaults_use_the_host_and_compiler_defaults() {
    let args = run_args(&["run", "main.sl"]);

    assert_eq!(args.input, PathBuf::from("main.sl"));
    assert!(args.target.target.is_none());
    assert_eq!(args.optimization.opt_level, OptLevel::O0);
    assert!(!args.optimization.debug_info);
    assert!(args.runtime_archive.is_none());
    assert!(args.program_args.is_empty());
}

#[test]
fn run_accepts_build_options_and_forwarded_arguments() {
    let args = run_args(&[
        "run",
        "-O2",
        "-g",
        "--runtime-archive=seal_rt.a",
        "main.sl",
        "--",
        "--flag",
        "value",
    ]);

    assert_eq!(args.optimization.opt_level, OptLevel::O2);
    assert!(args.optimization.debug_info);
    assert_eq!(args.runtime_archive, Some(PathBuf::from("seal_rt.a")));
    assert_eq!(
        args.program_args,
        vec![OsString::from("--flag"), OsString::from("value")]
    );
}

#[test]
fn run_requires_one_input() {
    let error = parse(&["run", "a.sl", "b.sl"]).expect_err("run accepts one program");
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn run_rejects_artifact_options() {
    for rejected in [
        &["run", "--emit=obj", "main.sl"][..],
        &["run", "-o", "program", "main.sl"][..],
    ] {
        let error = parse(rejected).expect_err("run owns its temporary output path");
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }
}

#[test]
fn run_help_lists_forwarded_arguments_and_build_options() {
    let mut command = Cli::command();
    let help = command
        .find_subcommand_mut("run")
        .expect("run is a subcommand")
        .render_long_help()
        .to_string();

    for expected in ["Optimization:", "Target:", "Runtime:", "ARGS", "--"] {
        assert!(help.contains(expected), "run help is missing `{expected}`");
    }
}
