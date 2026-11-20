# Project manifests

A Seal project is rooted at one `seal.toml`. The root manifest may declare a
`[project]`, a `[package]`, or both; every other member declares a `[package]`.

```toml
# seal.toml (workspace root)
[project]
name = "acme"
language = "0.1"
members = ["crates/*"]
default-members = ["crates/app"]

# seal.toml (member)
[package]
name = "app"
version = "0.1.0"

[targets.server]
kind = "bin"
entry = "app::main"

[dependencies]
geometry = { path = "../geometry" }
```

## Project and package

`[project] name` names the workspace. `members` accepts globs relative to the
root; a glob that matches no package is `E8004`, and a member without a
`[package]` table is `E8005`. `default-members` selects the packages built by
commands without `--workspace`.

`[package]` requires a valid lowercase `name` and a semantic `version`.
`description`, `license`, `repository`, `readme`, `metadata`, `features`, and
`[package.api]` are reserved for the project and publishing layers.
`[package.metadata]` is free-form.

## Sources

`package.sources` is a list of `.sl` globs relative to the package root. The
default is `src/**/*.sl`. `package.exclude` removes matching files. Globs are
validated before matching: absolute paths, `..`, and symlinks that leave the
package are rejected. A glob that matches no file is `E8004`.

A file joins a module with a first-item `module app::util;` declaration. A file
without one contributes to the package root. Several files may be fragments of
one module.

## Targets

Without `[targets]`, the project infers one target:

- `src/main.sl` produces a `bin` target with `entry = "root::main"`;
- otherwise `src/lib.sl` produces a `lib` target;
- otherwise loading fails with `E8003` and a pointer to `seal new`.

An explicit target names a `kind` and, for `bin`, `example`, and `generator`,
an `entry` function path. A missing entry is `E2087`; an entry that does not
resolve to one top-level function is also `E2087`. A `lib` target cannot name
an entry. `sources` and `include` add globs; `include` is restricted to tests,
benches, and examples. `reuse`, `profile`, `runtime`, `linker`, `linker-script`,
and `output` are accepted by the manifest parser and carried into the target
model.

`seal build` selects `lib` and `bin` targets by default. `--all-targets` adds
tests, benches, and examples.

## Dependencies and the lockfile

This phase resolves `path` dependencies inside the project. The alias is the
name imports use:

```toml
[dependencies]
geometry = { path = "../geometry", version = "0.1" }
```

A missing path, a version that does not match, an edge that leaves the project,
and a dependency cycle are reported before compilation (`E8002`, `E8006`).
Registry and git edges are rejected explicitly rather than ignored.

`seal.lock` records every path package, its relative source, a checksum over
its files, and its resolved dependency edges. It is sorted, has no timestamps
or absolute paths, and is written only when its bytes change. `--locked` and
`--frozen` fail with `E8008` instead of rewriting a stale lockfile.

## API

`[package.api] expose` lists the module paths other packages may reach.
Exposure is closed over ancestors; the root must be listed as `"root"` when it
is public. When the table is absent, the root is exposed, which is the useful
default for a binary. A lib usually writes:

```toml
[package.api]
expose = ["root"]
```

## Profiles and environment

Profiles control `opt-level`, `debug`, `debug-assertions`, `lto`,
`codegen-units`, `strip`, and `incremental`; `inherits` starts from another
profile. `dev` and `release` are built in. An unknown profile is `E8009`.

`[env]` values may be written as a string or as a table with `value`,
`tracked`, and `secret`. The values are carried by the project model; using
them in the compiler's build key belongs to the cache phase.

## Diagnostics

| Code | Meaning |
|------|---------|
| `E8001` | Plan file is missing, malformed, or uses an unknown schema |
| `E8002` | Unsupported dependency kind |
| `E8003` | Invalid target |
| `E8004` | Source glob matched no files |
| `E8005` | Invalid or contradictory manifest |
| `E8006` | Dependency cycle |
| `E8007` | Feature resolution failure |
| `E8008` | Lockfile missing, stale, or malformed |
| `E8009` | Unknown or invalid profile |
| `E8010` | Structurally invalid compile plan |
| `E8011` | Project I/O failure |
| `E2087` | Target entry does not resolve |
| `E8103` | More than one runnable target |
