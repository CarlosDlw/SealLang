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

`target.when` accepts a typed `cfg` table (`os`, `arch`, `env`,
`pointer-width`, `any`, `all`, `not`). An inactive target stays in the project
model but is skipped for a build whose triple does not match.

An explicit target names a `kind` and, for `bin`, `example`, and `generator`,
an `entry` function path. A missing entry is `E2087`; an entry that does not
resolve to one top-level function is also `E2087`. A `lib` target cannot name
an entry. `sources` and `include` add globs; `include` is restricted to tests,
benches, and examples. `reuse`, `profile`, `runtime`, `panic`, `linker`,
`sysroot`, `linker-script`, `link-args`, and `output` are accepted by the
manifest parser and carried into the target model. `runtime = "none"` is a
freestanding target: no runtime archive is linked, the entry becomes `_start`,
and panics trap. A freestanding target must use `panic = "trap"`; the parser
rejects the contradictory combination with `E8060`.

`seal/targets/<triple>.toml` adds a full target specification. `seal target
add` validates and installs one, `seal target list` shows the host and project
targets, and `seal target info <TRIPLE>` prints the resolved TOML. A custom
specification can set `pointer-width`, `endianness`, `data-layout`, `c-abi`,
`runtime`, `panic`, `linker`, `sysroot`, and `linker-script`. A cross
executable without a declared linker is `E8062`; a cross executable that would
link the compiler host's runtime is `E6003`, so a cross target either uses
`runtime = "none"` or stays at object inspection with `seal check`. A malformed
specification is `E8060`. The built-in defaults match the triple when no file
exists.

`seal build` selects `lib`, `bin`, `staticlib`, and `cdylib` targets by
default. `--all-targets` adds tests, benches, and examples. A library target
(`lib`, `staticlib`, or `cdylib`) cannot name an `entry`. `staticlib` writes
`lib<name>.a` or `lib<name>.lib`; `cdylib` writes `lib<name>.so`, `lib<name>.dylib`, or
`<name>.dll`; both expose only `export "c"` functions.

A `generator` target also declares runtime `inputs`, the `outputs` it writes,
and the cache contract it must satisfy. Another target opts into those files
with `generated = ["generator-name"]`; a target that is not a generator cannot
declare `inputs` or `outputs`. A target links declared native dependencies with
`native = ["alias"]`. See [builds.md](builds.md) for the execution and cache
contract.

## Generators, native libraries, and tasks

```toml
[targets.table]
kind = "generator"
entry = "root::main"
sources = ["src/table.sl"]
inputs = ["data/*.txt"]
outputs = ["generated/table.sl"]

[targets.app]
kind = "bin"
entry = "root::main"
generated = ["table"]
native = ["math"]

[native.math]
libs = ["m"]

[native.zlib]
pkg-config = "zlib"
static = true

[tasks.ci]
description = "Check and build the release profile"
depends-on = ["lint"]
steps = [
  { program = "seal", args = ["check"] },
  { program = "seal", args = ["build", "--release"], timeout = 600 },
]

[tasks.lint]
steps = [{ program = "seal", args = ["check", "--locked"] }]
```

Generator output globs must name a directory so undeclared files can be
detected. `generated` may only name a generator in the same manifest, and a
generator cannot consume another generator's outputs. A native declaration is
either explicit (`libs`, `search-paths`) or a `pkg-config` package; an empty
declaration, an unknown alias, or a library name containing a path is `E8005`.
Task steps name a program and an argument vector, never a shell command; `cwd`
stays inside the project and `timeout` is in seconds. `depends-on` runs named
tasks first, in declaration order, and a cycle is rejected. Arguments after
`seal task <name> --` are appended to every step.

## Dependencies and the lockfile

The alias is the name imports use. A dependency is either a path inside the
project or a registry package:

```toml
[dependencies]
geometry = { path = "../geometry", version = "0.1" }
foo = "1.2"
bar = { version = "^3", package = "bar-package", features = ["std"] }
```

A missing path, a version that does not match, an edge that leaves the project,
a registry configuration without a trusted TUF root, and a dependency cycle are
reported before compilation (`E8021`–`E8024`, `E8006`). Git edges are rejected
explicitly rather than ignored. See [dependencies.md](dependencies.md) for
resolution, the lockfile, vendor, audit, and SBOM behavior, and
[registry.md](registry.md) for the wire protocol and trust chain.

A project with registry dependencies declares `[registry]` in the root manifest
and pins its trust root:

```toml
[registry]
url = "https://registry.example.com/"
tuf-url = "https://registry.example.com/tuf/"
tuf-root = "trusted/root.json"

[registry.policy]
allow-licenses = ["MIT", "Apache-2.0"]
```

Dependency edges may carry a `when` condition and a per-edge feature list:

```toml
[dependencies]
serde = { path = "../serde", features = ["derive"], optional = true }
windows = { path = "../windows", when = { os = "windows" } }
```

`when` is evaluated against the target being built, not against the host that
loads the project. An inactive edge is not visited by feature resolution.

## Features

Features are additive inside one package instance but are never unified across
the graph. Two dependents that request different sets receive two instances of
the same package, each with its own aliases and symbol names. Equal sets share
one instance.

```toml
[package.features]
default = ["std"]
std = []
derive = ["dep:serde", "serde?/derive"]
use-windows = ["dep:windows"]

[package.features.conflicts]
std = ["no-std"]
no-std = ["std"]
```

`dep:name` activates an optional dependency, `name/feature` activates the
dependency and requests one of its features, and `name?/feature` requests the
feature only when the optional dependency is already active. Unknown names,
conflicts, and more than eight feature variants of one package are `E8007`.
The limits are deliberate: they keep the instance graph explainable.

`--features` and `--all-features` select features of the root package;
`--no-default-features` suppresses its `default` feature. `seal tree` prints
the resulting instance graph and `seal why` explains each instance and the
edge that requested it.

`seal.lock` records every package with its source, a source checksum, the union
of its instance features and `cfg`, every concrete instance, and its resolved
dependency edges. Path sources are relative and hashed over their files; registry
sources carry the exact index checksum and provenance, and versions coexist when
they belong to different compatibility lines. The file is sorted, has no
timestamps or absolute paths, and is written only when its bytes change.
`--locked` and `--frozen` compare the same per-edge resolution and fail with
`E8008` instead of rewriting a stale lockfile. A v1 lockfile is migrated in
memory to v2.

After a successful build, resolved native dependencies are appended as
`[[native]]` tables with the package, alias, pkg-config name, static flag,
libraries, search paths, and extra linker flags. `--locked` compares them too:
a system library change that alters the result is `E8008` until the lockfile is
refreshed.

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
profile and may inherit a built-in. Only the project root owns profile
definitions. `dev`, `release`, `test`, and `bench` are built in. A target may
name a profile; an explicit `--profile` or `--release` overrides it. An unknown
profile is `E8009`.

Source items and imports may carry `cfg(feature("..."))`,
`cfg(target.os == "...")`, `cfg(debug_assertions)`, `cfg(test)`, and
the other typed conditions. Items, imports, and whole modules whose condition
is false are not lowered or typechecked. `cfg(env("NAME"))` is accepted only
for a value declared in `[env]`; an undeclared name is `E2084`.

`[env]` values may be written as a string or as a table with `value`,
`tracked`, and `secret`. Build actions hash the values they use; secret values
appear in cache records and logs only as digests.

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
| `E8020` | Registry configuration error |
| `E8021` | Dependency integrity check failed |
| `E8022` | Missing path dependency |
| `E8023` | Incompatible dependency version |
| `E8024` | Invalid dependency declaration |
| `E8025` | Invalid registry index |
| `E8026` | Dependency resolution failed |
| `E8027` | Unsafe package archive |
| `E8028` | Registry lock conflict |
| `E8029` | Invalid vendor tree |
| `E8030` | Dependency audit failed |
| `E8031` | SBOM generation failed |
| `E8032` | Registry query failed |
| `E8033` | Dependency version is yanked |
| `E8034` | Dependency license is not allowed |
| `E8035` | Dependency source is not allowed |
| `E8036` | Dependency advisory applies |
| `E8037` | Dependency provenance is missing |
| `E8038` | Registry response is too large |
| `E8039` | Inconsistent registry metadata |
| `E8080` | TUF verification failed |
| `E8081` | Registry namespace error |
| `E8082` | Published version is immutable |
| `E8083` | Registry publish rejected |
| `E8084` | Registry credentials unavailable |
| `E8085` | Offline registry data is missing |
| `E8086` | Package metadata mismatch |
| `E8087` | Provenance verification failed |
| `E8088` | Registry cache error |
| `E8089` | Corrupt registry cache entry |
| `E8008` | Lockfile missing, stale, or malformed |
| `E8009` | Unknown or invalid profile |
| `E8010` | Structurally invalid compile plan |
| `E8011` | Project I/O failure |
| `E8012` | Affected-path query failed |
| `E8013` | Graph focus not found |
| `E8014` | Selection explanation failed |
| `E8040` | Cache object or action record invalid |
| `E8041` | Sandbox policy or input violation |
| `E8042` | Generator output contract violated |
| `E8043` | Native dependency not resolved |
| `E8044` | Sandbox timeout |
| `E8045` | Build action or graph failure |
| `E8046` | Task failure |
| `E8047` | Two build outputs claim one path |
| `E2087` | Target entry does not resolve |
| `E8103` | More than one runnable target |
