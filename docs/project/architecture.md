# Project layer architecture

The project layer turns a directory of `seal.toml` manifests into deterministic
compile plans, cached build actions, queryable graphs, and publishable packages.
The compiler architecture is described in [`../architecture.md`](../architecture.md);
this page documents the boundary between the compiler and the project, build,
dependency, registry, test, documentation, and remote layers.

## Responsibilities

| Crate | Owns |
|-------|------|
| `seal-manifest` | The TOML grammar, unknown-key diagnostics, typed values, and span-carrying validation |
| `seal-project` | Member discovery, source globs, path and registry resolution, feature instances, profiles, lockfile, plans, graphs, and affected-path mapping |
| `seal-plan` | The versioned JSON compile plan consumed by the driver |
| `seal-cas` | Content-addressed storage, the action cache, locking, and pruning |
| `seal-sandbox` | The sandbox policy and its Linux implementation, with a portable fallback |
| `seal-build` | Action graph, cache keys, executor, generators, native probes, and tasks |
| `seal-toolchain` / `seal-target` | Pinned toolchains, target specifications, and linker selection |
| `seal-registry` / `seal-registry-server` | Sparse index, TUF, downloads, publishing, auditing, SBOM, namespaces, and the transparency log |
| `seal-api` / `seal-doc` | Public-API snapshots and the HTML/JSON documentation renderer |
| `seal-test` | Test discovery, isolation, filters, sharding, retries, reports, and benchmark comparison |
| `seal-remote` / `seal-remote-server` | REAPI v2 cache and execution, with integrity checks and local fallback |
| `seal-query` | Incremental queries, persistent cache, and watch sessions |
| `seal-cli` | Command surface and user interface; it never re-implements resolution or selection |

`seal` writes a `CompilePlan` and calls the driver; `sealc` remains the
low-level single-file driver. The CLI therefore has one resolution
implementation and the driver has one plan parser.

## Pipeline

```text
seal.toml
  -> manifest validation (spans, unknown keys, targets, features)
  -> member discovery and package graph
  -> path/registry resolution and lockfile comparison
  -> per-edge feature and cfg evaluation -> package instances
  -> profile and target selection
  -> one CompilePlan per selected target
  -> frontend -> CGUs -> link, plus generators and native probes
  -> materialized artifacts under seal/target/<profile>/
```

Resolution is deterministic: packages, instances, dependency edges, lockfile
entries, graphs, and metadata are sorted before they are compared, hashed, or
printed. A path dependency is relative to the project; a registry package
carries the index checksum and provenance from its source. `--locked` and
`--frozen` compare the same per-edge resolution and fail with `E8008` instead of
rewriting a stale lockfile.

## Selection

```text
explicit -p / positional targets
  -> --workspace or declared default-members
  -> --exclude
  -> package target filters (lib/bin/check, executable for run)
  -> declared generator dependencies
  -> one CompilePlan per selected target
```

A plan always contains the transitive dependencies it needs. Selecting `app`
compiles `app`'s library chain without adding those packages as top-level jobs.
A target that declares `generated` also pulls in its generator targets; `build`,
`run`, `check`, `test`, and `bench` materialize those outputs before planning
the consumer. Generator outputs are part of the consumer's compile plan and its
cache key.

## Graphs and affected paths

`Project::graph` projects the loaded project into one of four deterministic
graphs:

- `packages`: package nodes and resolved dependency edges;
- `targets`: target nodes, library edges, and generator edges;
- `actions`: target actions with build ordering edges;
- `modules`: per-file module declarations and imports, linked across packages by
  dependency alias.

Nodes and edges are sorted before output. Focus and reverse are graph
operations, not separate data models, so every format sees the same result.

`seal_project::affected` runs `git diff --name-only` and `git ls-files --others`
from the project root, then maps paths through package roots, target files,
generator inputs/outputs, manifests, the lockfile, and the toolchain. A package
change propagates to every dependent package; the compile plan supplies the
actual dependency order. `--run` feeds the resulting jobs through the same
`check` path as a normal command.

## Build, sandbox, and cache

The build layer is an action graph, not a set of ad-hoc commands:

```text
frontend      package instances -> one .sealmono archive
codegen-cgu   .sealmono + unit -> one object
link          objects + runtime + native libraries -> executable or library
generate      generator executable + inputs -> declared files
native-probe  pkg-config or explicit flags -> a cached probe result
```

The frontend runs analysis and monomorphization once and splits the resulting
LLVM module into code generation units. Each unit is optimized and cached
independently, so editing one function invalidates the frontend and the unit
that contains it while the remaining units and the link stay cache hits when
their bytes do not change.

Every action is keyed by a closed list of inputs: source digests, toolchain,
target, profile, relevant environment (secrets enter the key as a digest and are
never logged), native probe results, and the action kind. The executor verifies
objects on read, discards corrupt entries (`E8040`), and rebuilds them. Two
concurrent builds may race on the same content-addressed object; both bytes are
identical, so keeping either is correct.

Tasks, generators, the linker, and native probes run through the sandbox with an
explicit working directory, environment, and read/write set. Generator output
globs are a contract: a missing output, an undeclared file, or a collision with
a source is `E8042`; a read outside the declared inputs is `E8041`. Native
dependencies are declared, never discovered silently.

## Registry and trust

`seal-registry` uses a sparse index and PubGrub resolution. Before any byte is
used, the client verifies the trusted TUF root, targets, snapshot, and
timestamp, then the archive length and SHA-256 digest; a rollback, expiry, or
mismatch is `E8080`. `file://` registries are supported for local fixtures and
`http://` only with `allow-insecure = true` for a loopback test registry.

`seal vendor` copies every locked registry package into
`vendor/<name>-<version>`, verifies its checksum and tree digest, writes
`vendor/seal-vendor.toml`, and records the resolved lockfile. `--offline` and
`--frozen` then build from that tree without contacting the registry.

Publishing is immutable. `seal publish` builds a reproducible archive plus an
API snapshot, a CycloneDX SBOM, and a DSSE/in-toto provenance statement. The
reference server enforces namespaces, checks the archive against its manifest,
appends every publish and yank to an RFC 6962 Merkle log, and returns an
inclusion proof and signed tree head. `seal audit --require-provenance` verifies
the full chain.

## Documentation, API, and tests

Parse-time doc comments are stored as spans in `seal-ast`; HIR records the same
spans on definitions, fields, variants, and modules. The driver extracts text
and builds the API snapshot, including layout from `seal-layout` and impls from
HIR. `seal-doc` converts that snapshot to HTML or JSON. A doc comment can never
inject HTML: the renderer escapes text before applying its small Markdown
subset. `seal semver-check` classifies the same snapshot, so documentation,
registry metadata, and compatibility checks describe one public surface.

`seal test` discovers `test func` and `bench func` entries in unit, integration,
and benchmark targets, builds each harness through the action cache, and runs
every test in its own process. Filters, sharding, retries, timeouts, stress
runs, and JUnit/JSON reports are runner concerns; none of them changes target
selection or compilation.

## Remote cache and execution

`seal-remote` is a synchronous REAPI v2 client behind a private Tokio runtime.
Every remote value crosses a trust boundary: response digests and sizes are
verified, action results are checked against the local key, and capabilities
are matched before execution. Frontend and CGU actions may execute remotely;
link, generator, and native-probe actions stay local because they consume host
toolchains or package files outside their declared inputs.

A network, authentication, capability, or timeout failure disables remote access
for the rest of the build and continues from the local cache. An action that ran
remotely and failed is never retried locally. `--require-remote` makes any remote
failure fatal. The reference server stores verified blobs and results under
`--root`, survives restarts, requires a token unless `--anonymous` is explicit,
and can serve TLS.

## Determinism

Every query sorts its output. API snapshots normalize modules, items, fields,
variants, and implementations before hashing. Metadata is emitted as pretty JSON
with no timestamps or process-specific fields. Link and archive inputs are
ordered, debug paths are remapped, and path dependencies are relative. `seal
verify-repro` builds a self-contained project in two directories and compares
every artifact byte for byte; this is what makes cache keys, registry digests,
and CI reports stable across machines.

## Extension points

| Layer | Extension point | Contract that stays fixed |
|-------|-----------------|---------------------------|
| Compiler | New backends and targets | `CompilePlan` schema and CGU/object boundary |
| Build | New action kinds | `ActionKey` closed input list and content-addressed outputs |
| Sandbox | OS-specific isolation | The `Sandbox` policy interface |
| Registry | Mirrors and federated indexes | TUF verification and immutable versions |
| Remote | Multi-tenant endpoints | REAPI v2 messages and digest verification |
| Tooling | LSP, formatter, macros | `seal metadata`, `seal api`, and `QueryDb` |
