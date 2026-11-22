# Seal language guide

This directory teaches the language from the user's point of view. Start with
[`overview.md`](overview.md) for the philosophy, goals, and design principles;
the documents in the [parent directory](../) are the normative design and
compiler references, while the chapters here explain how to use the implemented
surface correctly, with complete rules, worked examples, and the contracts
behind unsafe operations.

## Chapters

| Chapter | Covers |
|---------|--------|
| [`overview.md`](overview.md) | Philosophy, goals, design principles, safe-code guarantees, accepted trade-offs, and the rules for extending the language |
| [`copy.md`](copy.md) | Opt-in `Copy` for structs and enums, generic bounds, fields and variants, `Drop`, references, patterns, loops, statics, and diagnostics |
| [`raw-pointers.md`](raw-pointers.md) | `*raw T` and `*raw mut T`, casts, arithmetic, fat pointers, `unsafe`, constants, provenance, diagnostics, and patterns |
| [`ffi.md`](ffi.md) | Complete guide to C interop: `extern "c"` functions, statics, structs, opaque types, variadics and `c"..."`, function pointers and callbacks, buffers, linking, the safety contract, constant evaluation, diagnostics, and the acceptance matrix |
| [`first-class-functions.md`](first-class-functions.md) | `func` and `unsafe func` types, closures and capture, `move`, `Fn`/`FnMut`/`FnOnce` bounds, coercion, drop and borrow rules, ABI, diagnostics, and limits |
| [`modules.md`](modules.md) | Multi-file packages, module declarations and fragments, absolute imports, visibility, the public API, `cfg` filtering, cycles, and the `--plan` driver |
| [`prelude.md`](prelude.md) | Plan-selected preludes, injected item exports, lookup precedence, opt-out, diagnostics, and versioning |
| [`stdlib.md`](stdlib.md) | `core`/`std`, `Option`/`Result`, canonical paths, re-exports, moves, and the core/std preludes |
| [`testing.md`](testing.md) | `test`/`bench` modifiers, implicit `cfg` gating, `assert`/`assert_eq`/`assert_ne`, entry-point rules, unit and integration boundaries, discovery, runner reports, and benchmark measurement |
| [`builtins.md`](builtins.md) | `panic`, `exit`, equality assertions, type-name and type-of queries, size and alignment queries, compile-time evaluation, and shadowing |

## Project guides

The language guide stops at one package. These chapters cover the project layer
that compiles, tests, documents, and publishes packages:

| Chapter | Covers |
|---------|--------|
| [`../project/manifest.md`](../project/manifest.md) | `seal.toml`, members, targets, sources, profiles, features, dependencies, lockfile, and API exposure |
| [`../project/cli.md`](../project/cli.md) | Every `seal` command, selection rules, metadata, doctor, and exit status |
| [`../project/builds.md`](../project/builds.md) | Action graph, content-addressed cache, sandbox, generators, native libraries, tasks, and remote execution |
| [`../project/dependencies.md`](../project/dependencies.md) | Resolution, lockfile v2, vendor, audit, SBOM, and license policy |
| [`../project/registry.md`](../project/registry.md) | Sparse index, TUF, publishing, namespaces, transparency log, and the reference server |
| [`../project/architecture.md`](../project/architecture.md) | How the project, build, registry, test, documentation, and remote layers fit together |
| [`../project/acceptance.md`](../project/acceptance.md) | The acceptance matrix and the command that proves each line |
| [`../migration.md`](../migration.md) | Moving from `sealc` and single-file programs to `seal` projects |

## Reading conventions

- Code blocks are fragments that show the smallest context needed for the
  rule. The complete programs are the files under [`examples/`](../../examples/)
  and the end-to-end example at the close of each chapter.
- `T`, `U`, and `V` stand for concrete types. `T` must be `Sized` whenever the
  operation stores or advances by whole values.
- `Same` in a method table means the receiver's pointer type, including its
  mutability and pointee.
- Error codes are stable. Run `sealc explain <CODE>` for the long diagnostic.
- The reference documents remain authoritative when a compiler invariant and a
  guide sentence could be read differently:
  [`../types.md`](../types.md), [`../memory-model.md`](../memory-model.md),
  [`../architecture.md`](../architecture.md), and [`../codegen.md`](../codegen.md).

## Example programs

Every chapter points at a runnable file under [`examples/`](../../examples/).
The acceptance suite compiles each numbered example at `-O0` and `-O2`, runs
it, and asserts the documented exit status. Project examples live in
directories next to them and are built and executed by the same suite.
