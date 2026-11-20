# The `seal` command line

`seal` discovers a project, builds a versioned compile plan, and invokes the
compiler in-process. `sealc` remains the low-level single-file driver.

## Creating a project

```console
$ seal new demo --yes
$ seal new library --template lib --vcs none
$ seal new workspace --template workspace
$ seal init --template bin
```

`new` refuses a non-empty directory. The `bin` template includes a root module
and a second file joined through `module app::util;`; the generated program
calls that module. `lib` has no `main`. `workspace` creates a root manifest and
two members with a path dependency. Every template writes `seal.toml`,
`seal.lock`, `src/`, `tests/`, and `.gitignore`; `--vcs git` additionally runs
`git init`.

## Building and checking

```console
$ seal build
$ seal build --release
$ seal build --workspace
$ seal build -p app --target x86_64-unknown-linux-gnu
$ seal check -p app
```

`build` selects libraries and binaries and writes them under
`seal/target/<profile>/`. `check` runs the same analysis and writes no artifact.
`-p` selects one package; `--workspace` selects every package. `--all-targets`
adds tests, benches, and examples. A positional target name selects one target.
`--profile` chooses a declared profile and `--release` is shorthand for the
built-in release profile.

The lockfile is refreshed before a build. `--locked` and `--frozen` compare it
with the current resolution and fail with `E8008` instead of changing it.

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

## Metadata and doctor

```console
$ seal metadata
$ seal metadata --no-deps
$ seal doctor
$ seal doctor --json --fix
```

`metadata` prints versioned, deterministic JSON containing the project, target
triple, packages, targets, module index, files, features, and dependency
aliases. It is enough for an editor to list modules and targets without running
code generation.

`doctor` checks the embedded toolchain, a linker on `PATH`, a writable
`seal/target`, and a current lockfile. Manifest warnings include unknown keys
and suggestions. `--fix` creates the cache directory and rewrites the lockfile
when possible; `--json` emits the same checks as one object.

## Driver integration

`seal` writes a `CompilePlan` JSON file and calls the driver. The low-level
equivalent is:

```console
$ sealc check --plan-file plan.json
$ sealc build --plan-file plan.json
$ sealc build --emit=api plan.json -o api.json
$ sealc build --emit=obj plan.json -o module.o
```

A plan names the target, profile, packages in dependency order, source files,
exposed modules, dependency aliases, cfg values, the entry function, and the
artifact path. Invalid JSON or an unknown schema is `E8001`; a graph that
violates the plan invariants is `E8010`. Neither can panic the driver. The
historical `sealc build file.sl` path is unchanged.

`--emit=api` writes a JSON snapshot of the package modules and public items
instead of code. It is a tool-facing snapshot, not a stable ABI.

## Exit status

`0` means success. `1` is a project or compilation error, `2` is a usage error,
and `101` is an internal compiler error. `seal run` returns the child's status.
