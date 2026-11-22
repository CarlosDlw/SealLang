# The `seal` command line

`seal` discovers a project, builds a versioned compile plan, and invokes the
compiler in-process. `sealc` remains the low-level single-file driver.

## Creating a project

```console
$ seal new demo --yes
$ seal new library --template lib --vcs none
$ seal new workspace --template workspace
$ seal new kernel --template kernel --vcs none --yes
$ seal new frontend --template compiler --vcs none --yes
$ seal init --template bin
```

`new` refuses a non-empty directory. The `bin` template includes a root module
and a second file joined through `module app::util;`; the generated program
calls that module. `lib` has no `main`. `workspace` creates a root manifest and
two members with a path dependency; `crates/app` is the default member, so
`seal build` at the root selects it without `--workspace`. Every template writes
`seal.toml`, `seal.lock`, `src/`, `tests/`, and `.gitignore`; `--vcs git`
additionally runs `git init`. The `kernel` template is freestanding: it writes
`runtime = "none"`, `panic = "trap"`, a `kernel.ld` entry script, and a
`_start` function that exits with 42 through a generated `exit` stub. It builds
and runs on the host without libc or the Seal runtime, which is the acceptance
shape for bare-metal targets. The `compiler` template writes a small multi-file
pipeline: `src/ast.sl`, `src/parser.sl`, and `src/eval.sl` are separate modules
and `src/main.sl` imports them by absolute path. It exits with 42 and is the
worked example for module boundaries and package visibility.

## Building and checking

```console
$ seal build
$ seal build --release
$ seal build --workspace
$ seal build -p app --target x86_64-unknown-linux-gnu
$ seal build --target aarch64-unknown-linux-gnu --release
$ seal check -p app
```

`build` selects libraries, binaries, static libraries, and shared libraries and
writes them under `seal/target/<profile>/`. `check` runs the same analysis and writes no artifact.
`-p` selects one package; without it, every declared `default-members` is
selected (or the root package). `--workspace` selects every package and
`--exclude <PACKAGE>` removes one from either selection. Dependencies of a
selected package are still compiled inside its plan, so `-p app` builds the
library chain `app` consumes. `--all-targets` adds tests, benches, and examples.
A positional target name selects one target.
`--profile` chooses a declared profile and `--release` is shorthand for the
built-in release profile. `--cfg test`, `--cfg bench`, and
`--cfg debug_assertions` force the corresponding boolean condition for
coverage builds; other names are rejected because the configuration is typed.

The lockfile is refreshed before a build. `--locked` and `--frozen` compare it
with the current resolution and fail with `E8008` instead of changing it.

A target that declares `generated = ["name"]` pulls in that generator target.
`build`, `run`, `check`, `test`, and `bench` materialize the declared outputs
before planning the consumer, so no command needs a separate generator step.

A repeated build is a cache hit. Actions are keyed by their input digests,
hashed environment, and command, so changing one source rebuilds exactly the
targets that consume it. `-j/--jobs` bounds parallel actions, `--timings`
prints one line per action, and `--no-cache` bypasses reads and writes. The
status of every action is printed (`Running`, `Cached`, or a corrupt-entry
warning) unless `--quiet` or `--message-format=json` suppresses human output.
See [builds.md](builds.md) for the action graph, cache layout, sandbox,
generators, native dependencies, and tasks.

## Cache and cleaning

```console
$ seal cache stats
$ seal cache verify
$ seal cache prune --max-size 1073741824 --older-than 604800
$ seal cache explain <KEY>
$ seal clean
```

`cache verify` hashes every object and validates every action record, reports
corrupt paths with `E8040`, and exits `1` when something is wrong; the next
build discards and rebuilds those entries. `prune` removes old records and
least-recently-used objects. `explain` prints the action kind, display command,
input paths and digests, hashed environment, and output digests; environment
values are never printed. `clean` removes `seal/cache` and `seal/target`.

## Reproducibility

```console
$ seal verify-repro
$ seal verify-repro --keep
```

`verify-repro` copies a self-contained project into two temporary directories,
builds both, and compares every file under `seal/target` byte by byte. User
configuration under `seal/` (`targets` and pinned `toolchains`) is copied, while
the generated `seal/target` and `seal/cache` trees are not. It exits `1` with the
first differing artifact and byte offset (`E8104`). `--keep` leaves the two trees
in place for inspection. Path dependencies outside the project root are not
copied, so the check targets self-contained projects.

Project builds pass `--remap-path-prefix <project-root>=.` to the frontend, so
debug paths do not contain the checkout directory. The same inputs therefore
produce identical bytes under different roots.

## Tasks

```console
$ seal task ci
$ seal task ci --no-fail-fast
$ seal task ci -- --verbose
```

`task` runs a `[tasks.<NAME>]` recipe without a shell. `depends-on` names other
tasks that run first, in declaration order; a cycle is rejected. Each step names
a program and an argument vector, and arguments after `--` are appended to every
step. Steps run through the sandbox with an explicit working directory,
environment, and optional timeout. A failing step stops the task unless
`--no-fail-fast` is given. The task exits `1` when any step failed (`E8046`).

## Running

```console
$ seal run
$ seal run --bin server -- --port 8080
$ seal run -p app example
```

`run` needs exactly one executable target. A package with several binaries is
ambiguous (`E8103`); `--bin` or an explicit target name selects one. Generators
are excluded from the default executable selection, because they produce files
rather than a program; naming one explicitly is allowed. A target that declares
`generated` still runs its generators before the program is built. Arguments
after `--` are forwarded unchanged, and the process exit status is `seal`'s
status.

## Dependencies and registry

```console
$ seal add foo@1.2 --yes
$ seal add @scope/foo@^2 --features std --dry-run
$ seal remove foo --yes
$ seal update
$ seal update foo --precise foo@1.2.0
$ seal vendor --clean
$ seal audit --json
$ seal deny
$ seal sbom --output sbom.cdx.json
$ seal search serde --json
$ seal info serde
```

`add` and `remove` edit the selected package manifest with `toml_edit` and
require `--yes`; `--dry-run` prints the proposed graph without writing anything.
Both then resolve, write `seal.lock`, and run `seal check`, rolling the manifest
and lockfile back if the check fails. The global `--offline`,
`--minimal-versions`, and `--vendor` flags apply to dependency commands as well.
`update` refreshes every registry edge or only the named packages;
`--precise package@version` pins one selection and `--dry-run` leaves the
lockfile untouched.

`vendor` resolves the current graph when no lockfile exists, copies every locked
registry package into `vendor/` with checksum and tree verification, writes the
lockfile, and verifies that the new tree loads with `--offline` before
reporting success. `--clean` recreates the tree. `audit` (also available as
`deny`) checks advisories, licenses, sources, yanked versions, checksums, and
provenance; `--json` emits a report. `--require-provenance` fetches each
registry package's provenance, SBOM, API snapshot, and transparency proof and
fails when any of them is missing or invalid. `sbom` writes CycloneDX 1.5 JSON
to stdout or `-o/--output`. `search` and `info` query the signed catalog and
package indexes and support `--json`. See [dependencies.md](dependencies.md) and
[registry.md](registry.md).

## Publishing

```console
$ seal api
$ seal api --json -p app
$ seal semver-check 1.2.0
$ seal semver-check path/to/previous.api.json
$ seal publish --dry-run
$ seal publish -p app --signing-key signing.key
$ seal publish --workspace --allow-dirty
$ seal yank @scope/app@1.2.0
$ seal yank @scope/app@1.2.0 --undo
```

`api` prints the public snapshot or a summary with its digest. `semver-check`
classifies the current snapshot against a published version or a local file as
breaking, minor, or patch, prints every change, and exits `1` when a breaking
change is present (`E8091`). `publish --dry-run` validates license, readme,
sources, and signature, then prints the archive contents, size, API diff, SBOM,
and provenance without writing to the registry. A real publish refuses a dirty
Git tree (`E8095`), enforces the semver bump (`E8092`), and verifies the
registry's checksum, attestation digests, signed tree head, and inclusion proof
before reporting success. `--workspace` publishes in dependency order and skips
versions that already exist so an interrupted run can be repeated. The signing
key comes from `--signing-key`, `SEAL_SIGNING_KEY` (64 hex characters), or the
per-user `~/.config/seal/signing.key`, which is created with mode `0600` when
absent.

## Namespaces and login

```console
$ printf '%s' "$TOKEN" | seal login --token-stdin
$ seal login --registry http://127.0.0.1:8787 --allow-insecure --token-stdin
$ seal logout
$ seal owner @scope --claim --proof config
$ seal owner @scope --claim --proof dns --proof-value <TXT-VALUE>
$ seal owner @scope --add teammate
$ seal owner @scope --remove teammate
$ seal owner @scope --list
```

`login` validates the token with `GET`-equivalent `POST /api/v1/whoami` before
storing it, then writes the credentials file with mode `0600`. The token is read
from stdin with `--token-stdin` or from `SEAL_REGISTRY_TOKEN`; it is never echoed.
`logout` removes the stored token for the selected registry name. `owner` claims
or manages a namespace; a namespace that is not owned rejects publishes with
`E8083`. See [registry.md](registry.md) for the proof kinds, token scopes, and
the registry server configuration.

## Explaining the graph

```console
$ seal tree
$ seal tree -p app --features std
$ seal tree --duplicates
$ seal why -p serde --features std
$ seal why --file crates/app/src/main.sl
$ seal graph packages
$ seal graph targets --format dot
$ seal graph modules --format mermaid --focus app::root
$ seal graph actions --format json --reverse
$ seal affected --since origin/main
$ seal affected --since HEAD --run
```

`tree` prints the resolved package-instance graph. A package requested with two
different feature sets appears twice, each with its own bracketed feature list.
`--duplicates` lists packages with more than one selected version and exits
without printing the tree. `why` explains a package, feature, source file, or
target, including the dependency path from the primary package; `--json` emits a
versioned report. `why` uses `--target-triple` for cross compilation and
`--target` for the target name it explains.

`graph` prints `packages`, `targets`, `actions`, or `modules` as `ascii`
(default), `dot`, `mermaid`, or versioned `json`. `--focus` keeps a node and its
transitive descendants; `--reverse` follows edges in the opposite direction (or
reverses the whole graph when no focus is given). `affected --since REV` maps
`git diff` paths to packages and targets through the source graph and follows
reverse package dependencies; a manifest, lockfile, or toolchain change affects
the whole workspace. `--run` checks only the affected targets and `--json`
prints the report.

## Documentation and API

```console
$ seal doc -p app
$ seal doc -p app --format json -o target/doc
$ seal doc -p app --open
$ seal doc -p app --serve --address 127.0.0.1:8000
$ seal api -p app --json
$ seal semver-check 1.2.0 -p app
$ seal semver-check baseline-api.json -p app --json
```

`doc` renders outer `///` docs on items and fields, `/** ... */` docs, inner
`//!` module docs, signatures, `cfg` conditions, layouts, and trait/inherent
implementations. HTML goes below `target/doc` by default with one page per
module and links between them; `--format json` writes `index.json`. Private
items never appear because the renderer consumes the public API snapshot.
`--open` launches the platform browser and `--serve` starts a read-only static
server.

`api` prints the same versioned snapshot the registry hashes: modules, public
items, signatures, fields, variants, `cfg`, docs, layouts, and implementations.
`semver-check` compares against a published version or a snapshot file, prints
breaking/minor/patch changes, and exits `1` for a breaking change.

## Tests and benchmarks

```console
$ seal test
$ seal test --filter parser --skip slow --exact
$ seal test --partition hash:0/4 --retries 2 --timeout 30
$ seal test --stress 5 --no-fail-fast -j 8
$ seal test --list
$ seal test --format json
$ seal test --format junit -o reports/junit.xml
$ seal bench --iterations 1000 --samples 5 --save bench-baseline.json
$ seal bench --baseline bench-baseline.json --threshold 15
```

A function declared `test func` or `bench func` is a harness entry point. The
modifier is configuration-aware: test functions exist in `cfg(test)` builds and
benchmark functions in `cfg(bench)` builds, so ordinary builds neither type-check
nor emit them. `assert(condition)` and `assert(condition, "message")` are
available inside those functions; a failed assertion panics with the source
span and the optional message. A package without a `[targets]` table
automatically gets a `unit` target that reuses the library (`reuse = "lib"`,
with access to private items), one target per `tests/*.sl` integration file, and
one per `benches/*.sl` file. Integration targets depend on the library and see
only its public API.

`test` builds every selected test target and runs each test in its own process,
so a panic, signal, or timeout cannot take down the runner. `--filter` keeps
tests whose printed name (`module::name`, or just `name` in the package root)
contains the text and `--exact` requires a whole-name match; `--skip` removes
matches. The package and target are selected with the normal target/package
flags, not by the test name. `--partition
INDEX/TOTAL` (and `hash:INDEX/TOTAL`) selects one CI shard. `--retries`
reruns failures, `--timeout` kills a test and its process group, and `--stress`
repeats every selected test. Fail-fast is the default; `--no-fail-fast` reports
every result. Tests run with `-j/--jobs` in parallel.

`--format human` prints a colored line per test and a summary. `--format json`
writes one JSON object per result plus a final `summary` event as NDJSON, and
`--format junit` writes JUnit XML (default `seal/target/test-junit.xml`, or
`-o/--output`). A failing test makes the command exit `1`; `--list` prints the
selected names and exits without running anything.

`bench` uses the same discovery and filters, runs each benchmark body
`--iterations` times per process, and reports the fastest and mean wall time per
iteration across `--samples` processes. It makes no statistical claim beyond
that measurement. `--save` writes a stable JSON baseline; `--baseline` compares
against one and exits `1` when a benchmark is slower by more than `--threshold`
percent. `--json` prints the measurements and regressions as one JSON object.

## Metadata and doctor

```console
$ seal metadata
$ seal metadata --no-deps
$ seal doctor
$ seal doctor --json --fix
```

`metadata` prints versioned, deterministic JSON containing the workspace
(`name`, `root`, members, default members, lockfile state), target triple,
packages, package source and descriptive fields, targets (including required
features, generators, generated targets, and native aliases), the module index,
files, features, dependency edges with resolved names and versions, and concrete
package instances. It is enough for an editor or CI job to list, select, and
navigate the workspace without running code generation. `--no-deps` keeps only
the default package.

`doctor` checks the embedded or pinned toolchain, the host target, a linker on
`PATH`, sandbox isolation, the registry configuration, the root manifest, a
writable `seal/target`, the lockfile, and the disk used by `seal/`. A pinned
toolchain is verified against its installed tree; a missing or modified install
is `E8061`. The target check reports a declared linker, sysroot, or linker
script that does not exist, which would otherwise fail the link as `E8062`. The
sandbox check reports bubblewrap namespaces or the portable fallback, and the
registry check validates the URL and reports whether the configured token
variable is set. Manifest warnings include unknown keys and suggestions.
`--fix` creates the cache and target directories, repairs the lockfile, and
never changes source files; it never deletes data. `--json` emits the same
checks as one object with `version: 3`.

## Configuration, watch, completions, and lint

```console
$ seal config --list
$ seal config --key jobs --origin
$ seal config --json
$ seal watch --action test --filter parser
$ seal watch --once --action check
$ seal completions bash > seal.bash
$ seal completions zsh -o _seal
$ seal completions fish
$ seal completions powershell
$ seal man -o seal.1
$ seal lint --deny unused_import
$ seal lint --deny warnings
```

`config` resolves each key through one fixed order: command line, environment,
project manifest, user configuration, then the built-in default. The keys are
`target`, `profile`, `jobs`, `offline`, `registry.url`, `cache.dir`, `color`,
`message-format`, `remote.url`, `remote.execution`, `remote.require`,
`remote.instance`, `remote.timeout`, `remote.retries`, `remote.ca-cert`, and
`remote.tls-domain`; `SEAL_TARGET`, `SEAL_PROFILE`, `SEAL_JOBS`,
`SEAL_OFFLINE`, `SEAL_REGISTRY_URL`, `SEAL_CACHE_DIR`, `SEAL_COLOR`,
`SEAL_MESSAGE_FORMAT`, and the corresponding `SEAL_REMOTE_*` variables supply
the environment layer. The bearer token is read only from `SEAL_REMOTE_TOKEN`
and is never part of resolved configuration output. The user file is
`$SEAL_CONFIG`, or `$XDG_CONFIG_HOME/seal/config.toml`, or
`~/.config/seal/config.toml`. `--origin` prints the winning layer, `--json`
prints `{key, value, origin}` objects, and `--key` narrows the output.

`watch` polls `.sl` files, `seal.toml`, and `seal.lock` and reruns `build`
(default), `check`, or `test` after the tree has been quiet for `--debounce`
milliseconds. It uses the normal action cache, so an unchanged dependency is a
cache hit. The terminal is cleared before each run when stdout is a terminal;
`--once` runs the action immediately and exits, which is the scriptable and
testable form. Polling avoids a platform-specific notification dependency and
never rebuilds in a loop. Ctrl-C clears the terminal status and exits `130`.

`completions` generates bash, zsh, fish, or PowerShell scripts from the same
clap metadata as the parser. `man` renders a troff page with every command and
global option. Both write to stdout unless `-o/--output` names a file.

`lint` applies the compiler's warning policy instead of reimplementing lint in
the CLI. The stable names are `warnings` (the whole group) and `unused_import`;
`--deny` makes a match fail the command with the compiler's diagnostic, while
`--allow` validates a name for scripts. Unknown names are `E8112`. `--json`
prints `{status, denied_lints, allowed_lints}`. Test-only lints are checked by
`seal test`, which compiles the `cfg(test)` bodies.

## Toolchains and targets

```console
$ seal toolchain list
$ seal toolchain checksum path/to/bundle
$ seal toolchain install --from path/to/bundle --offline
$ seal toolchain install --from path/to/bundle --public-key <HEX>
$ seal toolchain pin 0.1.0
$ seal toolchain update --offline
$ seal toolchain verify 0.1.0
$ seal toolchain which
$ seal target list
$ seal target add target.toml
$ seal target info aarch64-unknown-none --json
```

The toolchain store lives at `seal/toolchains/<version>`. Installation copies
only regular files, rejects symlinks and paths outside the bundle, verifies the
SHA-256 tree digest before and after copying, and writes the install marker only
after both checks pass. A manifest may also carry an Ed25519 `signature` over
its canonical channel, version, checksum, components, and targets; such a
bundle requires `--public-key <HEX>` and an invalid signature is `E8061`. `pin`
writes the same manifest to `seal-toolchain.toml`;
`update` selects the highest installed semantic version. Both are offline by
default. A bundle's metadata names `channel`, `version`, `checksum`,
`components`, and `targets`; use `seal toolchain checksum` when authoring one.

`seal target list` always includes the host and adds every
`seal/targets/*.toml`. `add` validates and installs a specification under the
triple in the file; `info` prints the resolved TOML or a stable JSON object.
Target specifications are documented in [manifest.md](manifest.md#targets).

## Output policy

Status lines go to stdout and artifact paths are indented below them. `--quiet`
suppresses status without suppressing JSON from `metadata`, `doctor --json`, or
a program run. `--message-format=json` also suppresses human status so stdout
stays machine-readable.

Color is automatic only when stdout is a terminal. `NO_COLOR` and `--color
never` disable it; `--color always` keeps ANSI output even when redirected.
Errors and warnings always go to stderr.

## Driver integration

`seal` writes a `CompilePlan` JSON file and calls the driver. The low-level
equivalent is:

```console
$ sealc check --plan-file plan.json
$ sealc build --plan-file plan.json
$ sealc build --emit=api plan.json -o api.json
$ sealc build --emit=obj plan.json -o module.o
$ sealc build --emit=staticlib math.sl -o libmath.a
$ sealc build --emit=cdylib math.sl -o libmath.so
```

A plan names the target, profile, packages in dependency order, source files,
exposed modules, dependency aliases, cfg values, the entry function, and the
artifact path. Invalid JSON or an unknown schema is `E8001`; a graph that
violates the plan invariants is `E8010`. Neither can panic the driver. The
historical `sealc build file.sl` path is unchanged.

`--emit=api` writes a JSON snapshot of the package modules and public items
instead of code. It is a tool-facing snapshot, not a stable ABI.
`--emit=staticlib` and `--emit=cdylib` produce C libraries; the C surface is
the set of `export "c"` functions. Without `-o`, their default names are
`lib<input>.a`/`.lib` and `lib<input>.so`/`.dylib`/`<input>.dll`.

## Exit status

`0` means success. `1` is a project or compilation error, `2` is a usage error,
and `101` is an internal compiler error. `seal run` returns the child's status.
