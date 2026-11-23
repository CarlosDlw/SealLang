# Editions and deprecation

Seal separates the language version from the compatibility line a package was
written against. `project.language` names the implemented surface (`0.1`);
`[package] edition` names the line that may change meaning in a future breaking
release. The current edition is `2026` and changes no syntax today.

The normative policy is [RFC 0001](../rfcs/0001-editions-and-deprecation.md).
This chapter is the user guide: where to write the edition, how to mark a
declaration deprecated, what the compiler warns about, and how to control the
warning in a build.

## Selecting an edition

Write the edition in the package manifest:

```toml
[package]
name = "app"
version = "0.1.0"
edition = "2026"
```

`edition` is optional; when it is absent, the package uses the current edition.
`seal new` and `seal init` write it explicitly. `seal metadata` reports it for
every package, and a compile plan carries one edition per package. A dependency
keeps the edition it was written against; the dependent does not re-edition it.

An unknown year is `E8005` in a manifest and `E8001` in a plan. Version `2`
plans have no edition field, so they load with the current edition in memory;
version `3` plans carry the per-package value.

The edition is not the compiler release and not `project.language`. A manifest
may select `edition = "2026"` and still be compiled by a later compiler; the
language version stays exact and rejects an unimplemented `0.2`.

## Marking a declaration deprecated

`deprecated(...)` is a contextual modifier, like `test`, `bench`, and `public`.
It is consumed only at the start of a declaration, so `deprecated` remains a
valid identifier everywhere else.

```seal
deprecated(
    since: "0.2.0",
    note: "the current implementation replaces it",
    replacement: "current",
)
public func old_api() -> i32 {
    0
}

struct Config {
    deprecated(since: "0.2.0", replacement: "new_field")
    old_field: i32,
    new_field: i32,
}
```

Entries may appear in any order:

- `since` is required exactly once and must be a full semantic version such as
  `0.2.0`.
- `note` and `replacement` are optional, each at most once.
- Values are non-empty UTF-8 strings of at most 256 bytes, without control
  characters.

Unknown keys, duplicate keys, a missing or invalid `since`, an empty value, a
control character, and an over-long value are `E1001`.

Deprecation is accepted on functions, constants, statics, structs, enums,
opaque types, traits, trait methods, inherent methods, fields, variants, and
extern functions and statics. It is rejected on modules, imports, extern blocks,
and impl blocks with `E2079`. Positional tuple fields have no name to point at
and cannot carry it.

## What a use warns about

Using a deprecated item emits one warning at the use site:

```text
warning[deprecated]: `old` is deprecated since 0.1.0
  --> src/main.sl:15:5
   |
15 |     old() + 2
   |     ^^^ this use is deprecated
 --> src/main.sl:6:6
  |
6 | func old() -> i32 {
  |      --- the function was deprecated here
   = note: the current implementation replaces it
   = help: use current
```

The primary label is the use, the secondary label is the declaration, and the
optional note and replacement are guidance. The warning does not change the
program's behavior or exit status.

The forms covered by the same warning include value paths, type paths, trait
bounds and impl headers, type arguments, imports and re-exports, struct
literals, enum variants, patterns, field access, inherent methods, and trait
methods called through an implementation or `dyn Trait`. A use inside the
deprecated declaration itself is suppressed, so a recursive deprecated function
does not warn on every line of its own body. A `cfg`-inactive use is never
lowered and does not warn.

The warning uses the stable lint name `deprecated`. It is independent from
`unused_import`, so allowing one does not allow the other.

## Controlling the warning

`seal lint` accepts individual names and the `warnings` group:

```console
$ seal lint --project . --allow deprecated
$ seal lint --project . --deny deprecated
$ seal lint --project . --deny warnings
```

`--allow deprecated` suppresses the warning; `--deny deprecated` turns every
occurrence into an error and fails the command. `--deny warnings` covers every
warning the compiler emits, including `unused_import` and `deprecated`. The
`warnings` group cannot be allowed; allow individual lints instead.

`sealc` exposes the same control directly with repeatable flags:

```console
$ sealc check main.sl --allow-lint deprecated
$ sealc check main.sl --deny-lint deprecated
$ sealc check main.sl --deny-warnings
```

An unknown name is `E8112` in `seal lint` and a usage error before compilation
in `sealc`. Naming the same lint on both sides is rejected.

## API, documentation, and removal

The metadata is part of the public surface. `seal api` writes a `deprecated`
object with `since`, `note`, and `replacement` for each public deprecated item,
and `seal doc` renders the badge with escaped text. Layout, ABI, constant
evaluation, and emitted code are unchanged.

Deprecation is not removal. An item stays available and warns for at least one
minor release line. Removal happens only in an edition named by an accepted
RFC, with a release note and a migration recipe. The compiler never turns
`deprecated` into a hard error by itself; a project opts into that with
`--deny deprecated`.

The runnable [deprecation example](../../examples/deprecation/) prints the
warning and exits `42`, which keeps the example in the acceptance suite without
weakening the warning.
