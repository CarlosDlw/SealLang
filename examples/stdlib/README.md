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
They expose no methods and the language has no `?` operator.
`std::option::Option` and `core::option::Option` name the same type.

Borrowed strings live in `std::str`. Their indices are byte offsets, and the
safe `get`/`slice` pair validates UTF-8 boundaries. `std::str::from_utf8`
validates external bytes before producing a `*str`, while
`std::str::from_utf8_unchecked` is an `unsafe func` for already validated
bytes.

```console
$ seal run --bin stdlib --project .
$ echo $?
0
$ seal test --project .
```

The example's tests live in `tests/prelude.sl` and `tests/str.sl`; they cover
`Some`, `None`, `Ok`, `Err`, string ranges, UTF-8 validation, and safe view
construction.
