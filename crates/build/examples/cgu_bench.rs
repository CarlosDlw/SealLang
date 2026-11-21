//! Measures the split pipeline with one and several code generation units.
//!
//! The example builds the same synthetic project in two separate roots, times
//! several clean builds, compares the programs' exit status, and prints the
//! median wall-clock duration. It is deliberately not a unit test: timing depends on
//! the host and must never turn into a flaky assertion in the regular suite.
//!
//! Run it from the workspace with:
//!
//! ```console
//! cargo run --release -p seal-build --example cgu_bench
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use seal_build::{BuildOptions, Builder, JobRequest};
use seal_project::{PlanOptions, Project};
use seal_sandbox::Sandbox;

const RUNS: usize = 3;

struct Root(PathBuf);

impl Root {
    fn create(name: &str, units: usize, functions: usize) -> std::io::Result<Self> {
        let root =
            std::env::temp_dir().join(format!("seal-cgu-bench-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(
            root.join("seal.toml"),
            format!(
                "[package]\nname = \"bench\"\nversion = \"0.1.0\"\n\n\
                 [profiles.dev]\ncodegen-units = {units}\nopt-level = 3\n"
            ),
        )?;
        std::fs::write(root.join("src/main.sl"), source(functions, 0))?;
        Ok(Self(root))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn binary(&self) -> PathBuf {
        self.0.join("seal/target/dev/bench")
    }

    fn edit(&self, functions: usize, first: usize) -> std::io::Result<()> {
        std::fs::write(self.0.join("src/main.sl"), source(functions, first))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn functions() -> usize {
    std::env::var("SEAL_BENCH_FUNCTIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(800)
}

fn source(functions: usize, first: usize) -> String {
    let mut source = String::new();
    for index in 0..functions {
        let value = if index == 0 { first } else { index };
        source.push_str(&format!("func f{index}() -> i32 {{\n  {value}\n}}\n\n"));
    }
    source.push_str("func main() -> i32 {\n  let mut total = 0;\n");
    for index in 0..functions {
        source.push_str(&format!("  total = total + f{index}();\n"));
    }
    source.push_str("  total\n}\n");
    source
}

fn job() -> JobRequest {
    JobRequest {
        package: 0,
        target: "bench".to_owned(),
        options: PlanOptions {
            target: seal_target::host_triple().as_str().to_owned(),
            ..PlanOptions::default()
        },
    }
}

fn build(root: &Root, jobs: usize, cache: bool) -> (Duration, usize) {
    let project = Project::load(root.path()).expect("the benchmark project loads");
    let builder = Builder::new(
        &project,
        BuildOptions {
            jobs,
            use_cache: cache,
            sandbox: Sandbox::detect(),
            cache_root: None,
            link_args: Vec::new(),
            progress: None,
        },
    );
    let report = builder
        .build(&[job()])
        .expect("the benchmark build succeeds");
    (report.duration, report.misses())
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn measure(name: &str, units: usize, jobs: usize, functions: usize) -> (Duration, Root) {
    let root = Root::create(name, units, functions).expect("the benchmark root is writable");
    // A warmup keeps one-time filesystem and dynamic-loader costs out of the
    // median while every measured build still starts from an empty cache.
    let _ = build(&root, jobs, false);
    let mut samples = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        samples.push(build(&root, jobs, false).0);
    }
    (median(samples), root)
}

fn measure_incremental(
    name: &str,
    units: usize,
    jobs: usize,
    functions: usize,
) -> (Duration, usize) {
    let root = Root::create(name, units, functions).expect("the benchmark root is writable");
    let _ = build(&root, jobs, true);
    let mut samples = Vec::with_capacity(RUNS);
    let mut misses = 0;
    for value in 1..=RUNS {
        root.edit(functions, value)
            .expect("the benchmark source is writable");
        let (duration, count) = build(&root, jobs, true);
        samples.push(duration);
        misses += count;
    }
    (median(samples), misses / RUNS)
}

fn run(binary: &Path) -> ExitStatus {
    Command::new(binary)
        .status()
        .expect("the benchmark binary runs")
}

fn main() {
    let jobs = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let units = jobs.min(8);
    let functions = functions();
    let (single, single_root) = measure("single", 1, jobs, functions);
    let (split, split_root) = measure("split", units, jobs, functions);
    let (single_incremental, single_misses) =
        measure_incremental("single-edit", 1, jobs, functions);
    let (split_incremental, split_misses) =
        measure_incremental("split-edit", units, jobs, functions);
    let single_status = run(&single_root.binary());
    let split_status = run(&split_root.binary());
    println!(
        "{functions} functions, {jobs} jobs, {RUNS} runs (median)\n\
         clean:       1 CGU {single:?}; {units} CGUs {split:?}; {clean:.2}x\n\
         one edit:    1 CGU {single_incremental:?} ({single_misses} miss); {units} CGUs {split_incremental:?} ({split_misses} misses); {incremental:.2}x\n\
         same exit status: {same}",
        clean = single.as_secs_f64() / split.as_secs_f64(),
        incremental = single_incremental.as_secs_f64() / split_incremental.as_secs_f64(),
        same = single_status == split_status,
    );
    assert_eq!(
        single_status, split_status,
        "the split must preserve the program's behavior"
    );
}
