# Multi-module

This package shows the module surface without a dependency graph:

- `src/main.sl` has no `module` declaration and becomes the package root.
- `src/math.sl` and `src/math_extra.sl` both declare `module app::math;`; their
  items are merged into one module.
- `src/numbers.sl` declares `module app::numbers;`.
- Every import is absolute, starts at `root`, a top-level module, or a
  dependency alias, and is scoped to its file.

```console
$ cp -R examples/multi-module /tmp/multi-module
$ seal build --project /tmp/multi-module
$ seal run --project /tmp/multi-module
$ echo $?
42
```

`app` is an implicit parent module created by `module app::math;` and
`module app::numbers;`. No file declares `module app;`.
