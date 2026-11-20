//! Tests for the arguments of commands that analyze source files.

use std::path::PathBuf;

use clap::CommandFactory;
use clap::error::ErrorKind;
use seal_codegen::Emit;
use seal_session::{ErrorFormat, OptLevel};
use seal_target::TargetTriple;

use crate::cli::Cli;
use crate::cli::values::{EmitArg, ErrorFormatArg};

use super::{build_args, check_args, parse};

#[test]
fn build_defaults_match_the_compiler_defaults() {
    let args = build_args(&["build", "main.sl"]);
    assert_eq!(args.compile.inputs, vec![PathBuf::from("main.sl")]);
    assert_eq!(args.output.emit, EmitArg::Exe);
    assert_eq!(args.output.artifact(), Emit::Executable);
    assert_eq!(args.output.optimization.opt_level, OptLevel::O0);
    assert!(!args.output.optimization.debug_info);
    assert_eq!(args.compile.diagnostics.error_format, ErrorFormatArg::Human);
    assert_eq!(
        ErrorFormat::from(args.compile.diagnostics.error_format),
        ErrorFormat::Human
    );
    assert!(args.compile.target.target.is_none());
    assert!(args.compile.target.language_version.is_none());
    assert!(!args.compile.target.allow_unstable_features);
    assert!(args.runtime_archive.is_none());
}

#[test]
fn check_uses_the_same_analysis_options_without_output_options() {
    let args = check_args(&["check", "a.sl", "b.sl"]);
    assert_eq!(
        args.compile.inputs,
        vec![PathBuf::from("a.sl"), PathBuf::from("b.sl")]
    );
    assert!(args.compile.target.target.is_none());
    assert_eq!(args.compile.diagnostics.error_format, ErrorFormatArg::Human);

    for rejected in [
        &["check", "--emit=obj", "main.sl"][..],
        &["check", "-o", "out.o", "main.sl"][..],
        &["check", "-g", "main.sl"][..],
    ] {
        let error = parse(rejected).expect_err("check does not write artifacts");
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }
}

#[test]
fn optimization_levels_accept_attached_separate_and_bare_forms() {
    assert_eq!(
        build_args(&["build", "main.sl", "-O"])
            .output
            .optimization
            .opt_level,
        OptLevel::O3
    );
    assert_eq!(
        build_args(&["build", "-O2", "main.sl"])
            .output
            .optimization
            .opt_level,
        OptLevel::O2
    );
    assert_eq!(
        build_args(&["build", "--opt-level=1", "main.sl"])
            .output
            .optimization
            .opt_level,
        OptLevel::O1
    );
    assert_eq!(
        build_args(&["build", "--opt-level", "3", "main.sl"])
            .output
            .optimization
            .opt_level,
        OptLevel::O3
    );
    let error = parse(&["build", "-O9", "main.sl"]).expect_err("level 9 is not accepted");
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
}

#[test]
fn equals_and_separate_values_are_equivalent() {
    let attached = build_args(&["build", "--target=aarch64-unknown-linux-gnu", "main.sl"]);
    let separate = build_args(&["build", "--target", "aarch64-unknown-linux-gnu", "main.sl"]);
    assert_eq!(
        attached.compile.target.target,
        separate.compile.target.target
    );
    assert_eq!(
        attached.compile.target.target,
        Some(TargetTriple::parse("aarch64-unknown-linux-gnu").expect("test triple is valid"))
    );
}

#[test]
fn remap_specs_accumulate_in_order() {
    let args = build_args(&[
        "build",
        "--remap-path-prefix=/a=/b",
        "--remap-path-prefix=/c=/d",
        "main.sl",
    ]);
    assert_eq!(
        args.compile.diagnostics.remap_path_prefix,
        vec!["/a=/b", "/c=/d"]
    );
}

#[test]
fn double_dash_treats_remaining_arguments_as_inputs() {
    let args = build_args(&["build", "--", "--not-a-flag.sl"]);
    assert_eq!(args.compile.inputs, vec![PathBuf::from("--not-a-flag.sl")]);
}

#[test]
fn missing_values_are_reported() {
    let error = parse(&["build", "--target"]).expect_err("a target needs a value");
    assert!(error.to_string().contains("--target"));
}

#[test]
fn unknown_flags_are_rejected() {
    let error = parse(&["build", "--wat", "main.sl"]).expect_err("unknown flags are rejected");
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn build_requires_an_input() {
    let error = parse(&["build"]).expect_err("building needs an input");
    assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn link_options_are_repeatable_and_accept_leading_hyphens() {
    let args = build_args(&[
        "build",
        "-l",
        "sealfixture",
        "-lmath",
        "-L",
        "/tmp/lib",
        "-L/tmp/lib2",
        "--link-arg",
        "-Wl,--as-needed",
        "--link-arg",
        "extra.o",
        "main.sl",
    ]);
    assert_eq!(args.link.libraries, ["sealfixture", "math"]);
    assert_eq!(
        args.link.search_paths,
        [PathBuf::from("/tmp/lib"), PathBuf::from("/tmp/lib2")]
    );
    assert_eq!(
        args.link.link_args,
        [
            std::ffi::OsString::from("-Wl,--as-needed"),
            std::ffi::OsString::from("extra.o"),
        ]
    );
}

#[test]
fn runtime_archive_is_a_build_option() {
    let args = build_args(&["build", "--runtime-archive=seal_rt.a", "main.sl"]);
    assert_eq!(args.runtime_archive, Some(PathBuf::from("seal_rt.a")));
    assert_eq!(args.compile.inputs, vec![PathBuf::from("main.sl")]);
}

#[test]
fn codegen_flags_parse() {
    let args = build_args(&["build", "-g", "--emit=obj", "-o", "out.o", "main.sl"]);
    assert!(args.output.optimization.debug_info);
    assert_eq!(args.output.artifact(), Emit::Object);
    assert_eq!(args.output.path, Some(PathBuf::from("out.o")));

    assert_eq!(
        build_args(&["build", "--emit", "asm", "main.sl"])
            .output
            .artifact(),
        Emit::Assembly
    );
    assert_eq!(
        build_args(&["build", "main.sl"]).output.artifact(),
        Emit::Executable
    );
}

#[test]
fn output_requires_a_single_input() {
    let error = parse(&["build", "a.sl", "b.sl", "-o", "out"])
        .expect_err("one output cannot serve two inputs");
    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn invalid_emit_kinds_are_rejected() {
    for kind in ["llvm", "check"] {
        let error = parse(&["build", &format!("--emit={kind}"), "main.sl"])
            .expect_err("unknown artifact kind");
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }
}

#[test]
fn max_errors_must_be_positive() {
    let error = parse(&["check", "--max-errors=0", "main.sl"]).expect_err("zero is not a limit");
    assert_eq!(error.kind(), ErrorKind::ValueValidation);
    assert_eq!(
        check_args(&["check", "--max-errors=5", "main.sl"])
            .compile
            .diagnostics
            .max_errors,
        Some(5)
    );
}

#[test]
fn invalid_targets_and_versions_are_rejected_while_parsing() {
    assert_eq!(
        parse(&["build", "--target=plan9", "main.sl"])
            .expect_err("malformed triple")
            .kind(),
        ErrorKind::ValueValidation
    );
    assert_eq!(
        parse(&["check", "--language-version=next", "main.sl"])
            .expect_err("malformed version")
            .kind(),
        ErrorKind::ValueValidation
    );
}

#[test]
fn build_help_groups_options_by_concern() {
    let mut command = Cli::command();
    let help = command
        .find_subcommand_mut("build")
        .expect("build is a subcommand")
        .render_long_help()
        .to_string();
    for expected in [
        "Optimization:",
        "Output:",
        "Diagnostics:",
        "Target:",
        "Runtime:",
        "Link:",
        "--remap-path-prefix",
        "--runtime-archive",
        "--link-arg",
        "--library-path",
    ] {
        assert!(
            help.contains(expected),
            "build help is missing `{expected}`"
        );
    }
}
