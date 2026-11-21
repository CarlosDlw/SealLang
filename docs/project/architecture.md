# Project layer architecture

The project layer turns a directory of `seal.toml` manifests into deterministic
compile plans and queryable data. The compiler architecture is described in
[`../architecture.md`](../architecture.md); this page documents the project and
workspace boundary added in phase 10.

## Responsibilities

`seal-manifest` owns the TOML grammar and validation. `seal-project` owns member
discovery, path and registry resolution, the package graph, target selection,
the lockfile, plans, graph queries, and affected-path mapping. `seal-plan` owns
the versioned JSON document consumed by the driver. `seal-cli` is a thin shell
over those crates; it never re-implements resolution or selection.

`seal-doc` consumes only `seal_api::ApiSnapshot`, so documentation, the
registry, and `seal semver-check` describe exactly the same public surface.
`seal-api` keeps snapshots and compatibility classification independent from the
compiler and registry.

## Selection

```text
explicit -p / positional targets
  -> --workspace or declared default-members
  -> --exclude
  -> package target filters (lib/bin/check, executable for run)
  -> one CompilePlan per selected target
```

A plan always contains the transitive dependencies it needs. Selecting `app`
therefore compiles `app`'s library chain without adding those packages as
top-level jobs. `--exclude` resolves both package names and member paths.

## Graphs

`Project::graph` projects the loaded project into one of four deterministic
graphs:

- `packages`: package nodes and resolved dependency edges;
- `targets`: target nodes, library edges, and generator edges;
- `actions`: target actions with build ordering edges;
- `modules`: per-file module declarations and imports, linked across packages by
  dependency alias.

Nodes and edges are sorted before output. Focus and reverse are graph
operations, not separate data models, so every format sees the same result.

## Affected builds

`seal_project::affected` runs `git diff --name-only` and `git ls-files --others`
from the project root, then maps paths through package roots, target files,
generator inputs/outputs, manifests, the lockfile, and the toolchain. A package
change propagates to every dependent package; the compile plan supplies the
actual dependency order. `--run` feeds the resulting jobs through the same
`check` path as a normal command.

## Documentation and API

Parse-time doc comments are stored as spans in `seal-ast`; HIR records the same
spans on definitions, fields, variants, and modules. The driver extracts text
and builds the API snapshot, including layout from `seal-layout` and impls from
HIR. `seal-doc` converts that snapshot to HTML or JSON. A doc comment can never
inject HTML: the renderer escapes text before applying its small Markdown
subset.

## Determinism

Every query sorts its output. API snapshots normalize modules, items, fields,
variants, and implementations before hashing. Metadata is emitted as pretty JSON
with no timestamps or process-specific fields. This is what makes cache keys,
registry digests, and CI reports stable across machines.
