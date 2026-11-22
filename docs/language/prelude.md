# Prelude

A *prelude* is a module whose public items a package can use without writing an
import. The compiler does not choose that module from a hardcoded list: the
package plan names a `package` and a module path, and lowering reads the public
item exports of that module. The same mechanism therefore serves a synthetic
test prelude, the standard library, and a project-specific set of names.

```text
// core/src/option.sl
module option;
public enum Option<T> {
    Some(T),
    None,
}
```

```text
// app.sl
func main() -> i32 {
    let value = Option::Some(1); // no `use core::option::Option`
    match value {
        Option::Some(inner) => inner,
        Option::None => 0,
    }
}
```

## Selecting a prelude

A package plan has an optional prelude source:

```json
{
  "name": "app",
  "prelude": { "package": 0, "module": "root" }
}
```

`package` is an index into the plan's package list. `module` is an absolute
module path inside that package; `root` names the package root, and `a::b`
names a nested module. The source module must exist and must not require a
private module hop; a public module re-export is a valid path.

The standard library uses the `stdlib` form instead of a plan index:

```json
{
  "name": "app",
  "stdlib": "std",
  "prelude": { "stdlib": "std", "module": "prelude" }
}
```

The driver resolves `"std"` against the injected standard-library packages, so
the plan never stores an installation path. The shipped `core::prelude` and
`std::prelude` re-export `Option` and `Result` from `core::option` and
`core::result`. See [stdlib.md](stdlib.md) for the full library surface.

A package without a `prelude` field compiles exactly as before. The field is
optional in the versioned plan, so plans written before preludes existed remain
valid.

`core` and `std` do not receive a prelude of their own. Their plans leave the
field absent: they are the source of names, not consumers of the same list.
This keeps the bootstrap from making the prelude depend on itself.

## What is injected

Only **public item exports** of the source module enter the prelude:

- a public struct, enum, opaque type, trait, function, constant, static, or
  external declaration declared in the module;
- a public `public use` alias declared in the module, under its alias name.

Aliases keep canonical identity. A type exported as `Option` in the prelude is
the same type as the declaration it aliases; the prelude does not create a
second definition.

Modules are not injected. A `public use` that aliases a module is `E2101`,
because making a module name implicit would hide the explicit path shape the
language requires. Enum variants are not injected either: `Option` is in scope,
but `Some` is still written `Option::Some`. There is no glob import and no
syntax that expands one.

Items that are `internal` or `private` stay out of the prelude. A public alias
whose target is not public is already rejected by re-export validation.

## Precedence

Unqualified lookup uses this order:

1. the innermost local scope (a parameter, `let` binding, or capture);
2. an item declared in the current module;
3. an explicit import in the current file;
4. the package prelude.

The ordering is per namespace. A function named `Option` in the value namespace
does not hide the type `Option` in the type namespace, and a local type named
`Option` hides only the prelude type. Variants are still reached through the
type, so `Option::Some` works whenever `Option` resolves in the type namespace.

An explicit import of a name that also exists in the prelude is not a collision:
the import wins. This is true for item imports and for imports of a module. No
`E2078` is reported merely because the prelude offers the same name.

A prelude name is not an import. Using it never marks an import as used, and an
unused prelude name never emits `unused_import`. An explicit import that only
shadows a prelude name and is never used still emits the warning: the import
exists, so the compiler reports it.

## Opting out

The manifest field controls whether the project supplies the standard prelude:

```toml
[package]
name = "app"
version = "0.1.0"
implicit-prelude = false
```

The default is `true`. With `false`, the package plan carries no prelude source
and bare uses of prelude names fail as ordinary unresolved names.

Both tools also expose a command-line opt-out:

```console
$ seal build --no-prelude
$ sealc check --no-prelude --plan-file app.plan.json
```

`seal --no-prelude` prevents the project layer from adding the standard
prelude. `sealc --no-prelude` clears every prelude already named by a plan
file, which is useful for bootstrap builds and for checking a program without
its implicit names. The opt-out applies to tests and benchmarks exactly as it
applies to a normal target: they compile with the prelude selected for their
package.

## Diagnostics

| Situation | Code |
|-----------|------|
| The selected module does not exist in the source package | `E2100` |
| A public export of the prelude is a module | `E2101` |
| Two prelude exports bind the same name in one namespace | `E2102` |
| Package preludes form a cycle between different packages | `E2103` |

A colliding name is removed from the table rather than resolved to one of the
exports; an ambiguous name must be fixed, not guessed. A package may use its own
module as its prelude; only a cycle between different package sources is
`E2103`.

## Versioning

The plan schema keeps the prelude field optional and defaults it to absent.
Adding a prelude to a package is therefore a compatible plan change, while
changing the source module of an existing prelude changes which unqualified
names resolve and should be treated as a source-breaking change.

Within a package, adding a prelude name is a compatible addition unless it
shadows a name that callers already resolved some other way. Removing a
prelude name is source-breaking for every file that used it unqualified. The
language has no globs, so a prelude cannot silently grow to include a new
module or wildcard: every injected name is an explicit item or `public use`
export of the selected module.
