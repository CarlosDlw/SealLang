# Project examples

The numbered `*.sl` files in this directory are single-file programs compiled by
`sealc`. The directories below are complete `seal` projects that exercise the
module, manifest, build, dependency, and target layers. Each one is built and
run by the acceptance suite; copy it to an empty directory before building so
the example stays free of generated artifacts.

| Example | Demonstrates | Program result |
|---------|--------------|----------------|
| [`multi-module/`](multi-module/) | Module declarations, fragments, absolute imports, and package visibility | exits `42` |
| [`workspace/`](workspace/) | Root project, members, default members, path dependency, public API | exits `42` |
| [`features/`](features/) | Per-edge features, two instances of one package, `cfg(feature(...))` | exits `3` |
| [`generator/`](generator/) | Generator target, declared inputs/outputs, generated source compilation | exits `42` |
| [`freestanding/`](freestanding/) | `runtime = "none"`, linker script, `_start`, panic trap | exits `42` |
| [`registry/`](registry/) | TUF-verified registry dependency, vendoring, offline build | exits `42` |
| [`stdlib/`](stdlib/) | Hosted standard library, prelude, `Option`/`Result` matches, `core::str`, `mem::forget`, and tests | exits `0` |
| [`core-only/`](core-only/) | `stdlib = "core"`, core prelude, same canonical types | exits `0` |

```console
$ cp -R examples/multi-module /tmp/multi-module
$ seal run --project /tmp/multi-module
$ echo $?
42
```

`seal new --template compiler` scaffolds the same module layout used by a small
compiler front end (AST, parser, evaluator). The template is the recommended
starting point for a multi-file executable.
