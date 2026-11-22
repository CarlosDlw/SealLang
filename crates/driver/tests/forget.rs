//! Command-line acceptance for the `mem::forget` example.
//!
//! A direct call must disappear from the generated code and the program must
//! observe the suppressed destructor at both optimization levels. The assembly
//! check catches an accidental wrapper call or a leftover `forget` symbol.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_OUTPUT: AtomicU32 = AtomicU32::new(0);

struct TempOutput(PathBuf);

impl TempOutput {
    fn new(suffix: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "seal-driver-forget-{}-{}-{suffix}",
            std::process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/054_mem_forget.sl")
}

fn build(arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sealc"));
    command.args(arguments).arg(example());
    command.output().expect("the compiler runs")
}

#[test]
fn the_forget_example_runs_at_o0_and_o2() {
    for (arguments, suffix) in [
        (&["build"][..], "forget-debug"),
        (&["build", "-O2"][..], "forget-release"),
    ] {
        let output = TempOutput::new(suffix);
        let mut build_arguments = arguments.to_vec();
        build_arguments.extend(["-o", output.0.to_str().expect("utf-8 temp path")]);
        let compile = build(&build_arguments);
        assert!(
            compile.status.success(),
            "the example failed to build: {}",
            String::from_utf8_lossy(&compile.stderr)
        );

        let run = Command::new(&output.0).output().expect("the example runs");
        assert_eq!(
            run.status.code(),
            Some(0),
            "the example produced the wrong result"
        );
    }
}

#[test]
fn forgetting_a_droppable_type_collects_no_drop_glue() {
    let source = TempOutput::new("forget-only.sl");
    std::fs::write(
        &source.0,
        "struct ForgetOnly { value: i32 }\n\
         static mut COUNT: i32 = 0;\n\
         impl Drop for ForgetOnly {\n\
           func drop(self: *mut ForgetOnly) {\n\
             unsafe { COUNT += 1; }\n\
           }\n\
         }\n\
         func main() -> i32 {\n\
           std::mem::forget(ForgetOnly { value: 1 });\n\
           0\n\
         }\n",
    )
    .expect("the probe source is writable");
    let assembly = TempOutput::new("forget-only.s");
    let compile = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .arg("build")
        .arg("--emit=asm")
        .arg("-O0")
        .arg(&source.0)
        .arg("-o")
        .arg(&assembly.0)
        .output()
        .expect("the compiler runs");
    assert!(
        compile.status.success(),
        "the probe failed to compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let assembly = std::fs::read_to_string(&assembly.0).expect("the assembly is readable");
    assert!(
        !assembly.contains("seal.drop.glue"),
        "a type used only through forget must not collect drop glue"
    );
    assert!(
        !assembly
            .lines()
            .any(|line| line.contains("call") && line.contains("seal.ForgetOnly.drop")),
        "the destructor must not be called by a forgotten value"
    );
}

#[test]
fn a_direct_forget_emits_no_wrapper_symbol() {
    let output = TempOutput::new("forget.s");
    let compile = build(&[
        "build",
        "--emit=asm",
        "-O2",
        "-o",
        output.0.to_str().expect("utf-8 temp path"),
    ]);
    assert!(
        compile.status.success(),
        "the example failed to emit assembly: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let assembly = std::fs::read_to_string(&output.0).expect("the assembly is readable");
    let code = assembly
        .lines()
        .filter(|line| !line.trim_start().starts_with(".file"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("forget"),
        "a direct call must not leave a wrapper call or a forget symbol"
    );
}
