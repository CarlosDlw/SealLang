# Builds, the action cache, and hermetic tools

`seal build` is an action graph, not a sequence of compiler calls. Each action
has a content key; a repeated build with the same key reads the recorded result
from the local cache instead of running the command again.

```
frontend  one compile plan -> object + link metadata
link      object + runtime + native libraries -> executable
generate  generator executable + inputs -> declared files
native    pkg-config or explicit flags -> a cached probe result
```

The frontend and link actions are separate so re-linking after a cache hit does
not re-run analysis, and so a future CGU phase can insert more actions between
them without changing the cache contract.

## Where state lives

| Path | Contents |
|------|----------|
| `seal/target/<profile>/` | Materialized artifacts selected by targets |
| `seal/cache/objects/` | blake3-addressed bytes, sharded by the first two hex digits |
| `seal/cache/actions/` | One JSON record per action key |
| `seal/cache/locks/` | Short-lived advisory locks |
| `seal/cache/tmp/` | Atomic staging for objects, plans, and link outputs |

Cache directories are owner-only (`0700`) and objects are `0600`. Objects are
written to a unique temporary file, flushed, and renamed into place, so a
reader never observes a partial object. Two concurrent builds may write the
same object; both bytes are content-addressed, so keeping either is correct. A
remote or shared cache is deliberately out of scope in this phase.

## Action keys

An action key is the blake3 digest of a canonical JSON description containing:

- the action kind;
- every input file, sorted by project-relative path, with its content digest;
- the environment, as a map from name to the digest of the value;
- the display command;
- the keys of its dependencies.

Environment values are never stored as plaintext, in the record, in logs, or in
`--verbose` output. `seal cache explain <KEY>` prints the names and digests, the
command, and every output digest.

The frontend key includes the normalized compile plan: target, profile, package
instances, cfg values, entry, and every source file digest. Changing a source
therefore invalidates exactly the target that uses it; an unrelated file does
not change the key. The link key includes the object digest, the link metadata
digest, every native result, and the linker arguments.

## Cache behavior

A lookup verifies that every output object still exists and hashes to its key.
A malformed record, a missing output, or a corrupt object is a miss: the record
is removed, the action runs again, and the corrupt object is replaced. A build
therefore repairs its own cache instead of failing forever on one bad byte.

```console
$ seal build              # run actions and record results
$ seal build --no-cache   # bypass cache reads and writes
$ seal build --timings    # one line per action
$ seal build -j 8         # at most eight actions at once
$ seal cache stats        # object and record counts
$ seal cache verify       # hash every object and record
$ seal cache prune --max-size 1073741824 --older-than 604800
$ seal cache explain <KEY>
$ seal clean              # remove seal/cache and seal/target
```

`cache verify` reports corrupt paths and exits `1`; the next build discards and
rebuilds them. `cache prune` removes records older than `--older-than` seconds
and least-recently-used objects until the store fits `--max-size` bytes.

The number of jobs never changes the bytes of an artifact: independent actions
run in parallel, but each action's output is a pure function of its key. The
frontend currently serializes LLVM work inside the process until the split
pipeline of the next phase; links, generators, and probes still run in
parallel.

## Sandbox

External commands run through `seal-sandbox`. On Linux the backend uses
bubblewrap:

- an empty mount namespace exposes only the host toolchain directories and the
  paths named by the policy;
- inputs are read-only bind mounts, output directories are writable;
- `--unshare-all` removes the network unless the policy allows it;
- `--die-with-parent` and `--new-session` prevent an orphaned tool from
  surviving the build;
- the environment starts empty, so `HOME`, tokens, and locale settings never
  leak.

A wall-clock timeout kills the process and reports `E8044`. On a host without
bubblewrap, a policy may allow the portable fallback, which keeps the timeout
and empty-environment contract but cannot hide host files. The reported
isolation (`namespaces` or `portable`) is never silently upgraded.

## Generators

A generator is a host executable target that writes source files:

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
sources = ["src/main.sl"]
generated = ["table"]
```

A target lists the generators whose outputs it compiles. The build runs the
generator before planning that target, stores each output in the cache, and
adds the generated files to the compile plan. Generated files are never allowed
to overwrite a real source file.

Outputs are a contract:

- every declared output must exist after the command succeeds, or the build is
  `E8042`;
- every file below a declared output directory must match an output glob, or it
  is an undeclared output (`E8042`);
- an output that collides with a source, or with another generator, is
  `E8042`/`E8047` before the generator runs;
- changing a declared input or the generator executable changes the action key,
  so the generator reruns and the consumer is recompiled.

The generator receives `SEAL_GENERATOR_INPUTS`, `SEAL_GENERATOR_OUTPUTS`, and
`SEAL_GENERATOR_TARGET`. It runs in a staging tree that mirrors the package
layout: declared inputs and the executable are read-only mounts, output
directories are writable, and nothing else from the package is visible. Output
paths in `SEAL_GENERATOR_OUTPUTS` are absolute paths inside that staging tree.
After validation the build materializes the accepted files into the package and
stores them in the CAS; the staging tree is removed.

## Native dependencies

Native libraries are declared, never probed silently:

```toml
[native.math]
libs = ["m"]

[native.zlib]
pkg-config = "zlib"
static = true

[targets.app]
kind = "bin"
entry = "root::main"
native = ["math", "zlib"]
```

An explicit declaration contributes its `libs` and `search-paths`. A
`pkg-config` declaration runs `pkg-config --libs [--static] <name>` in the
sandbox and parses `-l`, `-L`, `-pthread`, and `-Wl,` flags. A missing package,
a missing pkg-config, or an unsupported flag is `E8043`. The result is cached
with the pkg-config executable digest, the declaration, and
`PKG_CONFIG_PATH`; it is also written to `seal.lock` after a successful build so
`--locked` covers native resolution. A changed result with `--locked` is
`E8008`.

## Tasks

Tasks are declarative steps without a shell:

```toml
[tasks.ci]
description = "check and build"
steps = [
  { program = "seal", args = ["check"] },
  { program = "seal", args = ["build", "--release"], timeout = 600 },
]

[tasks.release]
steps = [
  { program = "seal", args = ["build", "--release"], env = { SEAL_LOG = "info" } },
]
```

```console
$ seal task ci
$ seal task ci --no-fail-fast
```

Steps run in order through the sandbox with the project root writable, an empty
environment plus the declared values, an optional `cwd` relative to the project,
and an optional timeout. A failing step stops the task unless
`--no-fail-fast` is given; the task exit status is `1` when any step failed.
There is no implicit shell, so `>` and `&&` are arguments unless the program is
a shell the task explicitly names.

## Diagnostics

| Code | Meaning |
|------|---------|
| `E8040` | Cache object or action record invalid |
| `E8041` | Sandbox policy or input violation |
| `E8042` | Generator output contract violated |
| `E8043` | Native dependency not resolved |
| `E8044` | Sandbox timeout |
| `E8045` | Build action or graph failure |
| `E8046` | Task failure |
| `E8047` | Two build outputs claim one path |

## Current scope

The frontend action emits one object per target and the link action consumes it.
Code generation per CGU, the `.sealmono` intermediate, reproducible-link checks,
and staticlib/cdylib emission belong to the next phase; the action boundaries
and cache keys are already shaped for them. The single-file `sealc` path does
not use the CAS or the sandbox.
