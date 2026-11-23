# Migrating from `sealc` and single files

`sealc` is the low-level single-file driver and remains supported unchanged.
`seal` is the project layer: it discovers packages, resolves features and
dependencies, builds through a cached action graph, and drives the same
compiler. No Seal source needs to be rewritten to move from one to the other.

## What stays the same

```console
$ sealc build examples/006_structs.sl -o /tmp/structs
$ /tmp/structs; echo $?
10
```

A file without a `module` declaration is the package root. Its imports,
visibility, `cfg` conditions, and error codes are identical whether it is
compiled by `sealc` or as part of a `seal` package. Single-file programs keep
the same CLI and the same artifact behavior.

## Map the old invocation

| Old (`sealc`) | New (`seal`) |
|---------------|--------------|
| `sealc build file.sl` | `seal.toml` with `src/main.sl`, then `seal build` |
| `sealc build --plan main.sl util.sl` | same files under `src/`; `seal build` discovers `src/**/*.sl` |
| `sealc check --plan ...` | `seal check` |
| `sealc run file.sl` | `seal run --bin <name>` |
| `--expose app` / `--expose root` | `[package.api] expose = ["app"]` or `["root"]` |
| `-O0`/`-O2` flags | `[profile.dev]` / `[profile.release]`, `--profile`, `--release` |
| `-l m`, `-L dir` | `[native.math] libs = ["m"]` and target `native = ["math"]` |
| repeated `--plan` inputs | `package.sources` globs plus `package.exclude` |

The plan file remains the contract between the two layers:
`seal` writes a versioned `CompilePlan` and invokes the driver with
`--plan-file`. `sealc --plan` and `seal build` therefore compile the same
package graph; only the source of the plan changes.

## Add a manifest

For an empty directory, scaffold the project:

```console
$ cd my-program
$ seal init --template bin --vcs none   # writes seal.toml, src/, tests/, seal.lock
```

When the sources already live in `src/`, write the manifest by hand instead of
scaffolding, so `init` does not replace the existing files. A minimal manifest
for an existing executable is:

```toml
[project]
name = "my-program"
language = "0.1"

[package]
name = "my-program"
version = "0.1.0"
edition = "2026"
```

`edition` may be omitted; it then defaults to the current edition, `2026`.
`project.language = "0.2"` is not accepted by this compiler: `0.1` is the
implemented language surface, and a claimed `0.2` is `E8005` instead of being
silently compiled as `0.1`.

`package.sources` defaults to `src/**/*.sl`, and a package with `src/main.sl`
infers a `bin` target with `entry = "root::main"`. A library is `src/lib.sl`
with `[package.api] expose = ["root"]`.

```console
$ seal build
$ seal run
$ seal check
$ seal metadata > metadata.json
```

If the program is still one file, move it to `src/main.sl`. If it is already
split, keep every `module` declaration as it is: files are fragments of the
declared module, and imports are absolute exactly as with `--plan`. The only
change is that the manifest, not the command line, lists the files.

## Deprecate and migrate an API

A package removing a public item should first keep it with deprecation
metadata:

```seal
deprecated(
    since: "0.2.0",
    note: "the current implementation replaces it",
    replacement: "current",
)
public func old_api() -> i32 {
    0
}

public func current() -> i32 {
    42
}
```

Every use warns under the stable `deprecated` lint without changing behavior.
Suppress the warning during a staged migration with
`seal lint --project . --allow deprecated`; make the migration mandatory with
`--deny deprecated`, or deny every warning with `--deny warnings`. `sealc`
accepts the same policy through repeatable `--allow-lint` and `--deny-lint`
flags.

The declaration stays available until a removal edition is named by an accepted
RFC. Removing it earlier is a breaking change. The complete syntax, the
supported placements, and the API/documentation metadata are in
[`language/editions.md`](language/editions.md).

## Default standard library

A hosted package now receives the standard library by default. Hosted targets
default to `stdlib = "std"`; freestanding targets (`runtime = "none"`) default
to `stdlib = "core"`. Select it explicitly when the default is not wanted:

```toml
[package]
name = "my-program"
version = "0.1.0"
edition = "2026"
stdlib = "std"          # "std" | "core" | "none"
implicit-prelude = true # false keeps the dependency without implicit names
```

`std` injects `core` and `std`, `core` injects only `core`, and `none` installs
nothing. The `core` and `std` dependency aliases are reserved: an existing
package or dependency alias with either name is `E8136`. Rename that alias
before upgrading and update its imports.

The selected prelude puts `Option` and `Result` in scope, but variants remain
qualified (`Option::Some`, `Result::Ok`). There are no methods on either type
and no `?` operator. To keep the old unqualified behavior, set
`implicit-prelude = false` or pass `--no-prelude`. See
[`language/stdlib.md`](language/stdlib.md) and [`project/stdlib.md`](project/stdlib.md).

## Move a workspace

A directory of independent packages becomes a root project with members:

```toml
[project]
name = "acme"
language = "0.1"
members = ["crates/*"]
default-members = ["crates/app"]
```

Each member keeps its own `[package]` table. Replace `--plan` input lists with
path dependencies:

```toml
[dependencies]
geometry = { path = "../geometry" }
```

`seal build` at the root builds `default-members`; `--workspace` selects every
member and `--exclude` removes one. `seal graph`, `seal tree`, `seal why`, and
`seal affected` explain the resulting graph.

## Move dependencies

A registry dependency is a version requirement plus a pinned trust root:

```toml
[registry]
url = "https://registry.example.com/"
tuf-url = "https://registry.example.com/tuf/"
tuf-root = "trusted-root.json"

[dependencies]
foo = "~1.2"
```

```console
$ seal add foo@~1.2 --yes    # edits the manifest, resolves, writes seal.lock
$ seal vendor                # optional offline copy
$ seal audit                 # advisories, licenses, provenance
$ seal sbom --output sbom.cdx.json
$ seal build --offline --vendor
```

Commit `seal.lock` and the pinned `trusted-root.json`; never commit `vendor/`
unless the deployment requires it. `--locked` and `--frozen` make CI fail
instead of silently resolving a different graph.

## CI

| Before | After |
|--------|-------|
| `cargo build --release` then `./target/release/sealc build …` | `cargo build --release` then `./target/release/seal build --locked` |
| one job per `sealc` invocation | one `seal build --workspace` job |
| test script | `seal test --format junit -o reports/junit.xml` |
| manual cache directory | `seal cache` and optional `--remote` |

`seal verify-repro` builds a self-contained project twice in different
directories and compares artifacts byte for byte. `seal doctor --json` reports
the toolchain, target, linker, sandbox, registry, manifest, cache, lockfile, and
disk state before a pipeline runs.

## Compatibility guarantees

- `sealc build/check/run/explain` keep their historical inputs and semantics.
- A single-file program produces the same behavior and exit status.
- Module, import, visibility, `cfg`, and error codes are shared by both layers.
- A v1 lockfile is read and upgraded in memory; the next write stores v2.
- Reserved keywords and other breaking language changes are listed in
  [`RELEASES.md`](../RELEASES.md) with their source migration.

## Checklist

1. `seal init` (or a hand-written `seal.toml`) names the package and sources.
2. `seal check` passes with the same diagnostics as `sealc check --plan`.
3. `seal build` writes the expected artifact under `seal/target/dev/`.
4. `seal run` returns the program's exit status.
5. `seal.lock` is committed; `--locked` passes on a clean checkout.
6. `seal doctor` is green and `seal metadata` is the editor/CI input.
7. `seal verify-repro` passes before the pipeline is made blocking.
