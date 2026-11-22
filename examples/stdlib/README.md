# Standard library

This project selects `stdlib = "std"`. The `std` prelude puts `Option` and
`Result` in scope without an import, while their variants remain qualified:

```seal
func option_or(value: Option<i32>, fallback: i32) -> i32 {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}
```

`Option` and `Result` are not `Copy`; passing one to a function moves it.
There are no methods and no `?` operator. `std::option::Option` and
`core::option::Option` name the same type.

```console
$ seal run --bin stdlib --project .
$ echo $?
0
$ seal test --project .
```

The example's tests live in `tests/prelude.sl` and cover `Some`, `None`, `Ok`,
and `Err`.
