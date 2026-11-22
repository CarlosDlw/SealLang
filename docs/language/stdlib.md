# Standard library

The standard library is ordinary Seal source split into two packages:

| Package | Contents | Availability |
|---------|----------|--------------|
| `core` | `option`, `result`, `str`, an internal `char` helper, and the core prelude | dependency-free; no operating system or runtime |
| `std` | a facade over `core` and the hosted prelude | hosted targets |

A package chooses one mode in its manifest:

```toml
[package]
name = "app"
version = "0.1.0"
stdlib = "std"   # "std" | "core" | "none"
```

`std` injects `core` and `std`, `core` injects only `core`, and `none` injects
neither. Without the field, a hosted target defaults to `std` and a target with
`runtime = "none"` defaults to `core`. See
[`../project/stdlib.md`](../project/stdlib.md) for location, digests, cache,
and diagnostics. The `core` and `std` dependency aliases are reserved by the
compiler.

## Option and Result

`core::option` declares:

```seal
module option;

public enum Option<T> {
  Some(T),
  None,
}
```

`core::result` declares:

```seal
module result;

public enum Result<T, E> {
  Ok(T),
  Err(E),
}
```

Both are nominal enums. They are **not** `Copy`: assigning one, passing it by
value, or returning it moves the whole value. `match` is the only way to take a
payload out of a non-`Copy` variant. There are no methods, constructors, or
helpers on either type, and the language has no `?` operator.

Variants are always qualified by the enum:

```seal
func option_or(value: Option<i32>, fallback: i32) -> i32 {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}

func main() -> i32 {
  let some = Option::Some(41);
  let none = Option::<i32>::None;
  let ok = Result::<i32, i32>::Ok(1);
  let err = Result::<i32, i32>::Err(0);
  option_or(some, 0) + option_or(none, 0)
}
```

`Some`, `None`, `Ok`, and `Err` are never in scope by themselves. The prelude
puts the type names `Option` and `Result` in scope; it does not add the
variants.

## Strings

`core::str` owns the borrowed `str` surface. It exposes inherent methods for
substrings, UTF-8 scalar decoding, iteration, search, splitting, trimming, and
ASCII case comparison; `std::str` is a re-export of the same module, so both
paths name the same functions.

```seal
func main() -> i32 {
  let text: *str = "a🦀é";
  let crab = text.get(1, 5);
  let separator = text.find("🦀");
  let words = " one\ttwo ".trim().split_whitespace();
  0
}
```

All indices are byte offsets, and every returned view borrows its source. The
core operations are:

- `len`, `is_empty`, `is_char_boundary`, and `char_at` for scalar boundaries;
- `slice` (panics on an invalid range) and `get` (returns
  `Option<*str>` instead);
- `split_at`, `as_bytes`, `bytes`, `chars`, and `char_indices`;
- `starts_with`, `ends_with`, `contains`, and `find`; search is Two-Way with a
  worst-case `O(n + m)` bound;
- `split`, `split_once`, `lines`, and `split_whitespace`;
- `trim`, `trim_start`, and `trim_end` over Unicode whitespace;
- `eq_ignore_ascii_case`.

`from_utf8(*[u8]) -> Option<*str>` validates external bytes in place and returns
a borrowing view; `from_utf8_unchecked` is an `unsafe func` for bytes the caller
already validated. Neither constructor allocates. The full byte-index,
panic-message, empty-pattern, and line-ending rules live in
[`../types.md#string-methods`](../types.md#string-methods). The raw-pointer
`as_str` reinterpretation and its distinction from the safe constructor are in
[`raw-pointers.md`](raw-pointers.md#raw-string-pointers).

## Paths and identity

`core` exposes its `root`, `option`, `result`, `str`, and `prelude` modules.
`std` re-exports `core::option`, `core::result`, and `core::str` from its root,
so these paths all name the same canonical declarations:

```seal
core::option::Option<T>
core::result::Result<T, E>
core::str::from_utf8
std::option::Option<T>
std::result::Result<T, E>
std::str::from_utf8
```

The prelude aliases are canonical too:

```seal
core::prelude::Option<T>
core::prelude::Result<T, E>
std::prelude::Option<T>
std::prelude::Result<T, E>
```

A value of `core::option::Option<i32>` can be returned as
`std::option::Option<i32>` without a conversion, and the same holds for
`Result`. Re-exports are aliases, not new nominal types.

Explicit imports remain valid and take precedence over the prelude:

```seal
use core::option::Option;
use std::result::Result;
```

Importing the same name that the prelude provides is not a collision. The
import wins, and an unused import that only shadows a prelude name still
produces the usual unused-import warning.

## Prelude

`core::prelude` re-exports `Option` and `Result`. `std::prelude` re-exports the
same two names through `core`. A package with `implicit-prelude = true` (the
default) can therefore write `Option` and `Result` without an import in both
`std` and `core` modes:

```seal
func main() -> i32 {
  let value = Option::Some(1); // no `use` in std or core mode
  0
}
```

`implicit-prelude = false` and `--no-prelude` remove those implicit names while
keeping the library dependency. With `stdlib = "none"` or `--no-stdlib`, bare
`Option` and `Result` are unresolved unless the file declares or imports them.

## std as a facade

`std` is not a second copy of the core types. Its root re-exports the modules:

```seal
// std/src/lib.sl
public use core::option as option;
public use core::result as result;
public use core::str as str;
```

Its prelude re-exports the item aliases:

```seal
// std/src/prelude.sl
module prelude;

public use core::option::Option;
public use core::result::Result;
```

This keeps one definition of `Option` and `Result` in the program, whether a
file reaches them through `core`, `std`, or the prelude. It also keeps the
bootstrap honest: `core` and `std` select `stdlib = "none"` and
`implicit-prelude = false`, so neither library injects itself.

## Examples

- [`examples/052_strings.sl`](../../examples/052_strings.sl) is a self-contained
  program over the primitive string operations, including a checked UTF-8
  constructor.
- [`examples/stdlib`](../../examples/stdlib/) selects `std`, uses the hosted
  prelude, and exercises the `core::str` surface from its main program and
  tests.
- [`examples/core-only`](../../examples/core-only/) selects `core` and uses the
  same names without the hosted package.

Both projects run with `seal run --bin <name> --project .` and test `Some`,
`None`, `Ok`, and `Err` with `seal test --project .`. `--bin` selects the
program when the package also has test targets.
