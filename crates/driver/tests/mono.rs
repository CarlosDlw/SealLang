//! End-to-end tests for the `.sealmono` split pipeline.

use std::path::PathBuf;
use std::process::ExitCode;

fn fixture(name: &str, source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("seal-mono-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("fixture root");
    std::fs::write(root.join("main.sl"), source).expect("source");
    root
}

fn run(args: &[&str]) -> ExitCode {
    seal_driver::run_from(
        std::iter::once("sealc".to_owned()).chain(args.iter().map(|s| (*s).to_owned())),
    )
}

#[test]
fn mono_round_trip_emits_one_object_per_cgu() {
    let root = fixture(
        "round-trip",
        "func helper() -> i32 { 4 }\nfunc main() -> i32 { helper() }\n",
    );
    let mono = root.join("app.sealmono");
    assert_eq!(
        run(&[
            "build",
            "--emit=mono",
            "--cgu=2",
            "-o",
            mono.to_str().expect("path"),
            root.join("main.sl").to_str().expect("path"),
        ]),
        ExitCode::SUCCESS
    );
    let archive = seal_metadata::read_archive_file(&mono).expect("valid archive");
    assert_eq!(archive.manifest().cgu_count(), 2);
    assert_eq!(archive.modules().len(), 2);
    for cgu in 0..2 {
        let object = root.join(format!("app.{cgu}.o"));
        assert_eq!(
            run(&[
                "codegen",
                "--mono",
                mono.to_str().expect("path"),
                "--cgu",
                &cgu.to_string(),
                "-o",
                object.to_str().expect("path"),
            ]),
            ExitCode::SUCCESS
        );
        assert!(object.is_file());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_corrupt_artifact_is_e8070_not_an_ice() {
    let root = fixture("corrupt", "func main() -> i32 { 0 }\n");
    let mono = root.join("app.sealmono");
    assert_eq!(
        run(&[
            "build",
            "--emit=mono",
            "-o",
            mono.to_str().expect("path"),
            root.join("main.sl").to_str().expect("path"),
        ]),
        ExitCode::SUCCESS
    );
    let mut bytes = std::fs::read(&mono).expect("artifact");
    bytes[0] = b'X';
    let corrupt = root.join("corrupt.sealmono");
    std::fs::write(&corrupt, bytes).expect("corrupt artifact");
    assert_eq!(
        run(&[
            "codegen",
            "--mono",
            corrupt.to_str().expect("path"),
            "--cgu",
            "0",
            "-o",
            root.join("x.o").to_str().expect("path"),
        ]),
        ExitCode::from(1)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_out_of_range_cgu_is_rejected() {
    let root = fixture("cgu-range", "func main() -> i32 { 0 }\n");
    let mono = root.join("app.sealmono");
    assert_eq!(
        run(&[
            "build",
            "--emit=mono",
            "-o",
            mono.to_str().expect("path"),
            root.join("main.sl").to_str().expect("path"),
        ]),
        ExitCode::SUCCESS
    );
    assert_eq!(
        run(&[
            "codegen",
            "--mono",
            mono.to_str().expect("path"),
            "--cgu",
            "9",
            "-o",
            root.join("x.o").to_str().expect("path"),
        ]),
        ExitCode::from(1)
    );
    let _ = std::fs::remove_dir_all(root);
}
