# Core only

This project selects `stdlib = "core"`. Only `core` is injected; the hosted
`std` package and its operating-system dependencies are absent. The core
prelude still puts `Option` and `Result` in scope, with qualified variants:

```seal
func option_or(value: Option<i32>, fallback: i32) -> i32 {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}
```

`Option` and `Result` are not `Copy`, expose no methods, and there is no `?`
operator. `core::option::Option` is the canonical type also re-exported by
`std`.

```console
$ seal run --bin core-only --project .
$ echo $?
0
$ seal test --project .
```
