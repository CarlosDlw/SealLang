# The `seal` command line

`seal` discovers a project, builds a versioned compile plan, and invokes the
compiler in-process. `sealc` remains the low-level single-file driver.

## Creating a project

```console
$ seal new demo --yes
$ seal new library --template lib --vcs none
$ seal new workspace --template workspace
$ seal new kernel --template kernel --vcs none --yes
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
shape for bare-metal targets.

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
`-p` selects one package; `--workspace` selects every package. `--all-targets`
adds tests, benches, and examples. A positional target name selects one target.
`--profile` chooses a declared profile and `--release` is shorthand for the
built-in release profile. `--cfg test`, `--cfg bench`, and
`--cfg debug_assertions` force the corresponding boolean condition for
coverage builds; other names are rejected because the configuration is typed.

The lockfile is refreshed before a build. `--locked` and `--frozen` compare it
with the current resolution and fail with `E8008` instead of changing it.

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
```

`task` runs the steps of `[tasks.<NAME>]` in order without a shell and through
the sandbox. A failing step stops the task unless `--no-fail-fast` is given.
The task exits `1` when any step failed (`E8046`).

## Running

```console
$ seal run
$ seal run --bin server -- --port 8080
$ seal run -p app example
```

`run` needs exactly one executable target. A package with several binaries is
ambiguous (`E8103`); `--bin` or an explicit target name selects one. Arguments
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

`vendor` copies every locked registry package into `vendor/` with checksum and
tree verification, and `--clean` recreates the tree. `audit` (also available as
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
```

`tree` prints the resolved package-instance graph. A package requested with two
different feature sets appears twice, each with its own bracketed feature list.
`--duplicates` lists packages with more than one selected version and exits
without printing the tree. `why` prints every instance of a package, its
deterministic instance hash, its features, and the dependency edges that
requested it. Both commands accept
`--target`, `--features`, `--all-features`, `--no-default-features`, and
`--cfg`.

## Metadata and doctor

```console
$ seal metadata
$ seal metadata --no-deps
$ seal doctor
$ seal doctor --json --fix
```

`metadata` prints versioned, deterministic JSON containing the project, target
triple, packages, targets, module index, files, features, dependency edges, and
concrete package instances. It is enough for an editor to list modules and targets without running
code generation.

`doctor` checks the embedded or pinned toolchain, the host target, a linker on
`PATH`, a writable `seal/target`, and a current lockfile. A pinned toolchain
is verified against its installed tree; a missing or modified install is
`E8061`. The target check reports a declared linker, sysroot, or linker script
that does not exist, which would otherwise fail the link as `E8062`. Manifest
warnings include unknown keys and suggestions. `--fix` creates the cache
directory and rewrites the lockfile when possible; `--json` emits the same
checks as one object.

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
