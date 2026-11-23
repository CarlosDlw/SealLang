# Type aliases

`geometry` exposes primitive, generic, nominal, and enum aliases, and re-exports
the aliases declared in its `shapes` module. `app` imports those names across
the package boundary and uses them in signatures, constructors, generic
arguments, and patterns.

Aliases are transparent: the compiler expands every use to the target type, so
`PointAlias { x, y }` constructs the canonical `Point`, and `MaybeAlias::Some`
names the canonical variant.

```console
$ cp -R examples/type-aliases /tmp/type-aliases
$ seal check --project /tmp/type-aliases
$ seal build --project /tmp/type-aliases
$ seal run --project /tmp/type-aliases
$ echo $?
42
```

Inspect the published surface:

```console
$ seal api --project /tmp/type-aliases -p geometry --json
```

The snapshot lists `Score`, `Pair`, `PointAlias`, and `MaybeAlias` with
`kind: "type-alias"` and the `= <target>` signature. The re-exported names
carry the canonical declaration path, so consumers can distinguish an alias
from the underlying `Point` or `Maybe` type.
