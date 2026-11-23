# Continuous integration

The CI definitions live under `.github/`. They are templates: every job checks
out the repository, calls the shared setup action, and then runs one command
that proves the behavior it owns. A new gate belongs in the workflow whose
scope matches its cost, not in every workflow.

## Workflows

| Workflow | Triggers | Gates |
|----------|----------|-------|
| [`ci.yml`](../../.github/workflows/ci.yml) | Push to `main`, pull request, manual | Formatting, clippy, build, tests, reproducibility |
| [`acceptance.yml`](../../.github/workflows/acceptance.yml) | Push to `main`, nightly schedule, manual | Formatting, clippy, build, the serial test suite, reproducibility |
| [`install.yml`](../../.github/workflows/install.yml) | Push, pull request, manual | Shell syntax, shellcheck, the installer acceptance test |

`ci.yml` is the fast gate. Formatting runs first with only the Rust toolchain;
lint, build, tests, and reproducibility run in parallel once it passes. The
concurrency group cancels superseded runs on the same ref.

`acceptance.yml` is the expensive gate. It runs the suite with `-j 1
--test-threads=1`, which matches the acceptance matrix and catches tests that
depend on parallelism or shared state. It runs on `main`, every night, and on
demand, so pull requests are not blocked by a serial run.

`install.yml` builds the release binaries through `scripts/test-install.sh`.
That script compiles `seal` and `sealc`, so the job installs LLVM as well as
the Rust toolchain; shellcheck availability is part of the tested contract.

## Setup action

[`.github/actions/setup`](../../.github/actions/setup/action.yml) is the single
place that knows how to prepare a runner. It installs the pinned Rust
toolchain, restores the Cargo and target caches, and installs native tools on
demand.

| Input | Default | Effect |
|-------|---------|--------|
| `rust-toolchain` | `1.98.1` | Toolchain passed to `dtolnay/rust-toolchain`; `clippy` and `rustfmt` are always included |
| `llvm` | `true` | Installs LLVM 23, clang, lld, the LLVM system libraries, a C toolchain, and `pkg-config` |
| `bubblewrap` | `true` | Installs `bubblewrap` for sandbox tests |
| `shellcheck` | `false` | Installs `shellcheck` for the installer scripts |
| `cache` | `true` | Restores and saves the Rust caches |

Jobs select only what they need:

```yaml
- uses: ./.github/actions/setup
  with:
    llvm: "false"
    bubblewrap: "false"
    shellcheck: "true"
```

The LLVM step adds `/usr/lib/llvm-23/bin` to `PATH` and sets
`LLVM_SYS_231_PREFIX=/usr/lib/llvm-23`. Those names track the `llvm23-1`
feature in `inkwell` and the `llvm-sys` 231 line. When the LLVM pin changes,
update the package names, the `PATH` entry, `LLVM_SYS_231_PREFIX`, and the
`clang-23`/`llvm-split` checks in the setup action together.

Caching uses `Swatinem/rust-cache` with `cache-on-failure` enabled, so a failed
job still saves what it built. The cache key includes the job, the Rust
version, and the lockfile hash.

## Gates

Run the same commands locally before opening a pull request:

```console
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets --locked -- -D warnings
$ cargo build --workspace --locked
$ cargo test --workspace --locked
$ cargo test --workspace --locked -j 1 -- --test-threads=1
$ cargo run --locked -p seal-cli -- --project examples/multi-module verify-repro
$ scripts/test-install.sh
```

The first four commands and the reproducibility command are the fast gate; the
serial suite is the acceptance gate; the installer test is the installer gate.
The reproducibility command copies `examples/multi-module` twice, builds both
copies in different directories, and compares every artifact byte for byte. The
installer test builds release binaries, installs into temporary prefixes, and
checks the installed layout without `SEAL_STDLIB`.

`--locked` is required in every Cargo command. A CI run that updates
`Cargo.lock` is testing a different dependency set than the one the repository
records.

## Adding a gate

- Put a fast, deterministic check in `ci.yml`. It must finish well inside the
  job timeout and must not need a serial run.
- Put a slow, serial, or environment-sensitive check in `acceptance.yml`. A
  nightly failure is still actionable; a pull request blocked by a flaky
  nightly check is not.
- Put a check of the installer, shell scripts, or installed layout in
  `install.yml`.
- Reuse the setup action instead of duplicating toolchain installation. If the
  new job needs a native package, add an input or extend the dependency list
  there and document it in the table above.
