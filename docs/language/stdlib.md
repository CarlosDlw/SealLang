# Standard library

The standard library is ordinary Seal source split into two packages:

| Package | Contents | Availability |
|---------|----------|--------------|
| `core` | `option`, `result`, and the core prelude | dependency-free; no operating system or runtime |
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

## Paths and identity

`core` exposes its `root`, `option`, `result`, and `prelude` modules. `std`
re-exports `core::option` and `core::result` from its root, so these paths all
name the same canonical declarations:

```seal
core::option::Option<T>
core::result::Result<T, E>
std::option::Option<T>
std::result::Result<T, E>
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

- [`examples/stdlib`](../../examples/stdlib/) selects `std` and uses the hosted
  prelude.
- [`examples/core-only`](../../examples/core-only/) selects `core` and uses the
  same names without the hosted package.

Both examples run with `seal run --bin <name> --project .` and test `Some`,
`None`, `Ok`, and `Err` with `seal test --project .`. `--bin` selects the
program when the package also has test targets.
