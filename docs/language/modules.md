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

A plain import has no visibility of its own. Adding `private` or `test` to a
`use` is rejected as an invalid modifier (`E2079`).

## Prelude

A package plan may name one module as its *prelude*. The public item exports of
that module are then available in every file of the package without an import:

```text
// core.sl
public struct Option<T> { ... }

// app.sl
func main() { let value = Option { ... }; }
```

Only public items and public `public use` aliases are injected. Modules are not:
a module export in the prelude is `E2101`. Variants remain qualified, so the
prelude puts `Option` in scope but `Some` is still `Option::Some`, and there is
no glob syntax.

Unqualified lookup prefers, in order, the innermost local scope, an item of the
current module, an explicit import of the current file, and finally the
prelude. The order is per namespace, so a function named `Option` does not hide
a prelude type named `Option`. An explicit import that shadows a prelude name
wins without an `E2078` collision. Prelude names are not imports: they never
produce `unused_import`, and an unused explicit import that merely shadows a
prelude name still warns.

A package without a prelude compiles as before. The manifest field
`implicit-prelude` (default `true`) controls the standard prelude, and
`--no-prelude` disables it for both `seal` and `sealc`. The prelude source is
data from the compile plan; `core` and `std` themselves carry no prelude. A
missing module is `E2100`, two names in one namespace are `E2102`, and a cycle
between package preludes is `E2103`. See [prelude.md](prelude.md) for the full
rules and versioning notes.

## Re-exports

`public use` and `internal use` are re-exports. Unlike a plain import, they do
not bind a local name in the file. They make the target reachable through the
declaring module:

```text
// facade.sl
module app::facade;

public use app::util::double;
public use app::util::Point as Vec2;
public use app::util as utility;
```

Another file can now write `app::facade::double`, `app::facade::Vec2`, and
`app::facade::utility::double`. A re-export of a module exposes that module
under the alias and keeps the module's own children reachable below it.

A re-export is an alias, not a definition. Types keep their canonical identity:
`app::facade::Vec2` and `app::util::Point` are the same type to the type checker
and to the generated program, and re-exporting a function does not create a
second function. Because aliases can name other aliases, re-exports are resolved
to a fixed point before imports and bodies are lowered. Chains and diamonds are
valid; a path that only reaches other aliases and never a declaration is a
cycle.

Visibility narrows through a re-export, never widens:

- a `public use` needs its declaring module exposed, a public target item, and
exposed modules on the target path;
- an `internal use` is usable anywhere in its package, including targets that
are themselves `internal`, but a target from another package must already be
public and exposed;
- neither form may expose a `private` item.

A re-export cannot use a glob, `self`, or an enum variant. A path segment that
names an item where a module is required is `E2091`; re-export the module or
enum and reach the variant through that name instead. Module and item paths
also resolve through a re-export:

```text
func main() -> i32 { app::facade::double(21) }
```

| Situation | Code |
|-----------|------|
| A `public use` lives in a module the package does not expose | `E2088` |
| Re-exports form a cycle with no concrete target | `E2089` |
| Two re-exports declare the same name in one module | `E2090` |
| The path does not name a module or item that can be aliased | `E2091` |
| The target item is `internal` or `private` where the alias needs `public` | `E2092` |
| The target module is not exposed, or a hop crosses an internal alias | `E2093` |
| The alias collides with an item or submodule of the declaring module | `E2094` |

The API snapshot lists aliases separately from canonical items and records the
canonical target of each item alias. Changing which item an alias names is a
breaking API change even when the alias name is unchanged; adding an alias is a
compatible addition.

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

Exposure paths may end in a module re-export, which is how a facade such as
`std::option` is published. The alias must be a `public use` in an exposed
module, and its canonical target module must be exposed as well. Prefixes that
already exist are exposed before re-exports are resolved; the final alias is
resolved afterwards, so `[package.api] expose = ["std::option"]` and the
canonical target exposure can be configured together.

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

A project manifest replaces the explicit input list. `seal` discovers every
source through `package.sources`, assigns files to modules from their first
item, and passes the resulting package graph to the driver:

```console
$ seal check
$ seal build
$ seal run --bin app
```

See [`../project/manifest.md`](../project/manifest.md) for the manifest and
[`../project/cli.md`](../project/cli.md) for selection, profiles, and scripts.

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

`cfg(...)` is a typed condition evaluated before lowering. It may prefix an
item, a module declaration, or an import:

```text
cfg(feature("std")) internal func allocate() {}
cfg(target.os == "linux") module app::linux;
cfg(feature("extra")) use app::extra::Helper;
```

The expression grammar supports `feature("name")`, `target.os`, `target.arch`,
`target.env`, `target.pointer_width`, `debug_assertions`, `test`, `bench`,
`env("NAME")`, the comparison operators, `!`, `&&`, and `||` with the usual
precedence. An
`env` name must be declared in the project environment; an undeclared name is
`E2084`. Conditions on a field, variant, or trait method are rejected with
`E2085`.

An inactive item, module, or import is removed before name resolution, type
checking, and code generation. It produces no definition, no symbol, and no
`unused_import` warning; code inside it is never resolved, so it may name
items that do not exist in the current configuration. This is why two
`cfg`-exclusive declarations may share one name:

```text
cfg(feature("a")) public func which() -> i32 { 1 }
cfg(feature("b")) public func which() -> i32 { 2 }
```

Feature selection is per package instance. The project layer evaluates each
instance's feature set and target facts and passes the result to the module
graph; `seal why` prints the conditions an instance received. The expression
grammar and the project-level `cfg` table are documented in
[`../architecture.md`](../architecture.md) and
[`../project/manifest.md`](../project/manifest.md).

## Cycles

Module and import cycles are valid. Import resolution builds the module graph
before any body is lowered, so `app::a` may import `app::b` while `app::b`
imports `app::a`. A cycle across files is not an error and does not change
visibility or initialization order, because Seal has no module initializers.

Package cycles are a different rule: a dependency edge may not return to the
package that introduced it. `seal` reports `E8006` with the dependency path
before compilation starts. Split the shared code into a third package instead
of forming a cycle.
