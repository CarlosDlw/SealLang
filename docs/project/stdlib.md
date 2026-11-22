# The standard library

The Seal standard library is a directory of ordinary Seal packages described by
one `stdlib.toml`. The compiler locates and validates that directory, computes a
stable digest over its contents, and injects the selected packages into every
compile plan. The library is not a project dependency: it does not appear in
`seal.lock`, `seal vendor`, `seal tree`, or dependency metadata.

The initial release ships two packages:

- `core` has no operating-system, heap, or runtime dependency;
- `std` is the hosted facade and depends on `core`.

Both are compiled from source for each plan. There is no precompiled standard
library and no registry interaction.

## Layout

```text
stdlib/
├── stdlib.toml
├── core/
│   ├── seal.toml
│   └── src/
│       ├── lib.sl
│       └── prelude.sl
└── std/
    ├── seal.toml
    └── src/
        ├── lib.sl
        └── prelude.sl
```

Every package directory contains its own `seal.toml` and source tree. The
standard-library manifest sets `stdlib = "none"` and
`implicit-prelude = false` so the library never injects itself.

## Descriptor

```toml
format = 1
language = "0.1"
version = "0.1.0"
packages = ["core", "std"]
digest = "20b69aafa6f85f02632f3a9ca7a914acd8e78c2562e70234c8e998e49c9349c1"

[prelude]
std = "std::prelude"
core = "core::prelude"
```

- `format` versions the descriptor schema. This compiler understands format
  `1`; another format is `E8135`.
- `language` must be the language version the compiler implements (`E8134`).
- `version` is the standard-library semantic version.
- `packages` is the closed, dependency-ordered package list.
- `[prelude]` maps a mode's package name to its prelude module. The module path
  is written `package::module`.
- `digest` is the BLAKE3 digest of the descriptor fields, every package
  manifest, every source file, and every internal dependency edge. Absolute
  paths, modification times, owners, and permissions are excluded, so the same
  tree has the same digest after being copied or installed.

Unknown descriptor fields are rejected. A package name is lowercase ASCII
letters, digits, `_`, or `-`. Every package directory must be a real directory,
every manifest must name the directory it lives in, and every listed source must
be a regular UTF-8 file inside the standard-library root (`E8139`). Symlinks are
not followed. Internal dependencies are path dependencies between listed
packages; a duplicate package, a missing target, an unsupported option, a path
pointing at the wrong directory, or a cycle is `E8137`.

## Modes

A package selects a mode in its manifest:

```toml
[package]
name = "kernel"
version = "0.1.0"
stdlib = "core"        # "std" | "core" | "none"
implicit-prelude = true
```

- `std` injects `core` and `std` and selects `std::prelude`.
- `core` injects only `core` and selects `core::prelude`.
- `none` injects nothing and requires no installation.
- Without `stdlib`, a hosted target receives `std` and a target with
  `runtime = "none"` receives `core`.
- `stdlib = "std"` on a target with `runtime = "none"` is `E8138`.
- `implicit-prelude = false` keeps the library dependency but injects no names.
  `seal --no-prelude` does the same for one build.

Every package that selects a mode gains the `core` dependency alias, and a `std`
package also gains `std`. Those aliases are reserved: a user package or
dependency alias named `core` or `std` is `E8136`.

`sealc` selects the same modes for one file:

```console
$ sealc check main.sl                 # std
$ sealc check --core-only main.sl     # core
$ sealc check --no-stdlib main.sl     # none
$ sealc check --no-prelude main.sl    # std without implicit names
```

`--plan-file` uses the modes recorded in the plan and ignores `--core-only` and
`--no-stdlib`.

## plan v2

`CompilePlan` version 2 carries only the standard-library identity:

```json
{
  "version": 2,
  "stdlib": {
    "language": "0.1",
    "version": "0.1.0",
    "digest": "20b6..."
  },
  "packages": [
    {
      "name": "app",
      "stdlib": "std",
      "prelude": { "stdlib": "std", "module": "prelude" }
    }
  ]
}
```

The plan contains no absolute standard-library path and no sources. The driver
loads the local installation, verifies language, version, and digest, and only
then injects the packages. A user package's prelude may name another user
package (`"package": 0`) or a standard-library package (`"stdlib": "std"`).
Version 1 plans are read as `stdlib = "none"` with no prelude, preserving the
behaviour they were written for.

Injection preserves user package indexes: `seal tree` and every API surface see
the same user packages in the same order. Standard-library packages are placed
first in the HIR plan, user dependencies are remapped, and the primary package
index is adjusted. A plan that selects a mode without an identity, or names both
prelude sources, is `E8141`; a plan whose identity differs from the installed
library is also `E8141`.

## Location

The loader searches, in order:

1. `SEAL_STDLIB`, which must point at the directory containing `stdlib.toml`;
2. `<prefix>/lib/seal/stdlib/<language>`;
3. `<prefix>/share/seal/stdlib/<language>`;
4. a development fallback next to the workspace (`<repo>/stdlib`).

`<prefix>` is the parent of the directory containing the running executable.
The executable path is canonicalized first, so invoking a symlink does not
change the sysroot. An invalid `SEAL_STDLIB` is `E8133` and never falls back
silently.

Inspect the selected tree with:

```console
$ seal stdlib path
$ seal stdlib version
$ seal stdlib verify
```

`seal stdlib verify` recomputes the digest and reports `E8140` when a manifest
or source changed. `seal doctor` reports the root, language, version, digest,
packages, and the hosted and freestanding default modes.

## Cache and reproducibility

The standard-library digest is part of the compile plan and therefore of the
frontend action key. Changing any manifest or source changes the digest and
invalidates cached frontend and build actions; changing only a modification
time does not. The action also hashes every standard-library source file
explicitly, so a broken installation cannot serve stale bytes.

Standard-library diagnostics and debug locations use `<stdlib>/...` identities,
never the installation path. Two builds in different directories with the same
standard library are byte-identical inputs to the same action keys.

## Diagnostics

| Code | Meaning |
|---|---|
| `E8133` | The standard library could not be located. |
| `E8134` | The standard library targets another language version. |
| `E8135` | The descriptor is malformed or the digest does not match. |
| `E8136` | A user package or dependency alias uses `core` or `std`. |
| `E8137` | The package list, internal dependencies, or graph are invalid. |
| `E8138` | A mode conflicts with the target runtime. |
| `E8139` | A package, manifest, source, or path is invalid. |
| `E8140` | `seal stdlib verify` found a mismatch. |
| `E8141` | The plan and the installed standard library differ or contradict. |
| `E8142`–`E8149` | Reserved. |
