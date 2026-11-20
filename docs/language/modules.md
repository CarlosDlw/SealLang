# Modules, imports, and packages

A Seal program is a *package* made of one or more source files. Files are split
into *modules*, and every item belongs to exactly one module of exactly one
package. Modules and packages are what make a name unique: the function `run`
in `app::cli` and the function `run` in `app::serve` are different definitions
and never collide in the generated program.

This chapter describes the implemented module surface: how a file declares its
module, how imports bind names, how visibility is enforced, and how a package
exposes its public API. The normative rules live in
[`../architecture.md`](../architecture.md).

## Files and modules

A module is a semantic namespace, not a directory. A file joins a module by
starting with a declaration:

```text
module app::util;
```

The declaration must be the first item of the file. It may appear at most once,
its path must be non-empty, and it cannot take a visibility or `test` modifier.
A late, repeated, or empty declaration is `E2069`.

Parent modules are implicit. `module app::util;` creates `app` as a module even
when no file declares `app` alone, and the path `app::util` reaches `util`
through it. The package root is the empty path; it is written `root` where a
path needs to start there.

A file without a `module` declaration contributes to the package root. This is
why a single-file program needs no module declaration at all:

```text
func main() -> i32 { 0 }
```

Several files may declare the same module. Each file is a *fragment*, and the
fragments share the module's namespace:

```text
// util.sl
module app::util;
internal func double(value: i32) -> i32 { value * 2 }
```

```text
// fragment.sl
module app::util;
internal func extra(value: i32) -> i32 { value + 1 }
```

Both functions are items of `app::util`. Two fragments cannot declare the same
name: the duplicate is `E2083`, and the diagnostic points at both declarations.
An empty module is valid; a module exists as soon as some file declares it or
names it on the way to a module that is declared.

Module declarations are not imports and do not bring names into scope. A file
sees its own module's items by name, and anything else must be imported.

## Imports

An import brings one name, or a module, into the scope of the current file:

```text
use app::util::double;
use app::util::{double, extra};
use app::util::double as twice;
use app::util;
```

Import paths are absolute. The first segment is one of:

- `root`, the current package root;
- a top-level module of the current package, such as `app`;
- a dependency alias declared by the package plan, such as `geometry`.

There is no relative import and no `self` or `super`. Every following segment
must name a submodule until the last one, which may name a module or an item.
A group is expanded into one binding per name; `as` renames the local binding.

An import of a module binds the module, so it can start another path:

```text
use app::util;
func main() -> i32 { util::double(21) }
```

Imports are **file-scoped**. A name imported by one file is not visible in
another file, even when both files are fragments of the same module, and it is
not re-exported from the module. Importing it again in the other file is the
only way to use it there.

Imports are resolved before any body is lowered, so modules may import each
other in a cycle. What cannot happen is an import that does not resolve:

| Situation | Code |
|-----------|------|
| The first segment names no module or dependency | `E2073` |
| A later segment names no submodule | `E2070` |
| The last segment names no item of the module | `E2073` |
| A name denotes both a module and an item | `E2074` |
| Two imports bind the same local name | `E2078` |
| An import collides with an item or submodule of the file's module | `E2077` |
| The target item is not visible from the importing module | `E2072` |
| The target module belongs to another package and is not exposed | `E2071` or `E2080` |
| A glob (`use a::*;`) is written | `E2076` |
| An imported name is never used | `unused_import` warning |

An import has no visibility of its own. `public use` and `internal use` are
rejected as invalid modifiers (`E2079`); a module re-exports names by declaring
its own items, not by re-exporting imports.

## Qualified paths

The same absolute path shape works in expressions and types:

```text
func main() -> i32 { app::util::double(21) }
func make() -> app::util::Point { app::util::Point { x: 1, y: 2 } }
```

`root::name` reaches an item of the package root, including when the path has a
single segment; the parser treats `root` specially because it is always a
module and never a type.

The prefix resolves through the module graph, and the last segment resolves in
the namespace the position needs: value, type, or trait. `Type::associated`
keeps its existing meaning: when the first segment names a type rather than a
module, the path names an associated function or variant of that type, so
`Point::new()` and `Option::Some(1)` are unchanged.

A module is a namespace, not a value:

```text
func main() {
    let value = app; // E2086: a module is not a value
}
```

The compiler reports `E2086` instead of trying to give a module a type. A
module that does not exist in the current package is `E2070`; a segment the
module does not declare is `E2002`, exactly as for an unqualified name.

## Visibility

Every item, field, and inherent method may take a visibility modifier. The
default is `private`.

| Modifier | Visible from |
|----------|--------------|
| `private` (default) | the declaring module and its descendant modules |
| `internal` | anywhere in the declaring package |
| `public` | other packages, when the declaring module is exposed |

```text
module app::util;

private func helper() -> i32 { 1 }
internal func double(value: i32) -> i32 { helper() * value }
public func answer() -> i32 { 42 }
```

`helper` is reachable from `app::util` and from `app::util::detail`, but not
from `app` or from the root. `double` is reachable anywhere inside the package.
`answer` is reachable from another package only if `app::util` is exposed.

Visibility is checked at every import and every access, not only at the
declaration. A module is always internal to its package and cannot take a
visibility modifier; another package reaches it only through the package API.
Variants inherit the visibility of their enum, and trait methods inherit the
visibility of their trait, so neither accepts its own modifier.

Fields are checked independently. A public struct may keep private fields:

```text
public struct Point {
    public x: i32,
    private y: i32,
}
```

Other packages may read and write `x` but cannot name `y`; the struct remains
constructible inside the package. A public field whose type is not public is
`E3104`.

## The public API

A package chooses the modules other packages may reach. In a manifest this is
`[package.api] expose`; with the low-level driver it is `--expose`:

```text
// src/main.sl
func main() -> i32 { app::answer() }

// src/app.sl
module app;
public func answer() -> i32 { 42 }
```

```console
$ sealc check --plan --expose app src/main.sl src/app.sl
```

Exposing a module also exposes every ancestor needed to reach it, so
`--expose app::util` makes `app` reachable as well. Exposure must be explicit:
when at least one `--expose` is given, the root is not exposed unless `root` is
listed too. When no `--expose` is given, the driver exposes the root, which is
the right default for a binary crate.

`public` promises that an item is reachable from another package. That promise
is checked against the exposed set:

| Situation | Code |
|-----------|------|
| A `public` item lives in a module that is not exposed | `E2080` |
| The API names a module the package does not declare | `E2081` |
| A public function signature names a non-public type | `E3100` |
| A public constant or static names a non-public type | `E3101` |
| A public trait method names a non-public type | `E3102` |
| A public bound names a non-public trait | `E3103` |
| A public field names a non-public type | `E3104` |
| A public variant carries a non-public type | `E3105` |

Signature validation runs after every body is lowered, so it sees the resolved
types of aliases, generics, and associated functions. A `public` item that is
never reachable is therefore rejected at the declaration, not silently ignored
until a caller fails.

An implementation is allowed only in the package that declares its type or its
trait. A foreign implementation is `E2082`; wrap the foreign type in a local
struct when the behavior must live in the current package.

## Building multiple files

Without `--plan`, every input is an independent single-file program, which is
the historical behavior. With `--plan`, all inputs are compiled as one package:

```console
$ sealc check --plan src/main.sl src/app.sl src/util.sl
$ sealc build --plan -o program src/main.sl src/app.sl src/util.sl
```

The first input is the package's primary file: `build` derives the artifact
name from it, and code generation uses it as the entry file. Every other input
must parse, and all inputs share one module graph. The same package is checked
by `check`, `build`, and `run`; `run` remains a single-file command because it
takes one input.

## Complete example

```text
// main.sl
use app::util::{double, Point};

func main() -> i32 {
    let point = Point { x: 20, y: 22 };
    double(point.x) + point.y
}
```

```text
// util.sl
module app::util;

internal struct Point {
    internal x: i32,
    internal y: i32,
}

internal func double(value: i32) -> i32 { value * 2 }
```

```console
$ sealc check --plan main.sl util.sl
$ sealc build --plan -o example main.sl util.sl
```

`Point` and `double` are internal, so the root file can import them but no
other package can. No module declaration of `app` exists; it is created
implicitly by `module app::util;`. The two files are fragments of one package,
and the program exits with `62`.

## Configuration

`cfg(...)` conditions are parsed and retained on items and imports, including
the `cfg` written on a field, variant, or trait method that is rejected with
`E2085`. Selecting configuration is part of the project layer: lowering does not
yet filter declarations by `cfg`, so every parsed declaration is analyzed in
this phase. The expression grammar and its diagnostics are documented in
[`../architecture.md`](../architecture.md).
