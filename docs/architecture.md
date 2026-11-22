# Seal compiler architecture — foundation through code generation

This document covers the compiler foundation and every production phase: the
lexer, the parser, HIR lowering with name resolution, type checking, MIR
lowering, ownership/borrow checking, and LLVM code generation. The foundation is
the set of crates that every later phase depends on. It is kept in sync with the
code; changes to a public invariant must change this file in the same commit. The memory model that
the type representation encodes is decided in
[`memory-model.md`](memory-model.md).

## Workspace layout

The compiler is a Cargo workspace. Each crate has one responsibility and is
allowed to depend only on crates below it in the list.

| Crate | Responsibility |
|-------|----------------|
| `seal-index` | Typed, non-zero `u32` indices (`newtype_index!`) and `IndexVec`. |
| `seal-hash` | Fast in-memory hashing (`FxHashMap`) and the versioned stable hash (`StableHasher`, `Fingerprint`). |
| `seal-macros` | `#[derive(StableHash)]`. Proc macros live apart so the compiler does not pay for `syn` at every rebuild. |
| `seal-symbol` | Thread-safe string interner; `Symbol` is a 4-byte handle with integer equality. |
| `seal-unicode` | Identifier classification (XID) and terminal display width. |
| `seal-span` | `SourceMap`, `SourceFile`, `LineIndex`, `Span` (8 bytes), snippets, path remapping. |
| `seal-diagnostics` | Structured diagnostics, `ErrorGuaranteed`, dedup, rich/JSON/silent emitters, ICE. |
| `seal-error-codes` | Stable code registry (`E0xxx`–`E9xxx`) with long-form explanations. |
| `seal-lexer` | Single-pass tokenizer: tokens, trivia, numeric/string/char literals, lexical diagnostics. |
| `seal-ast` | Syntax tree: typed arena IDs, nodes with exact spans, and the `AstBuilder` the parser writes through. |
| `seal-parser` | Recursive descent for items and statements, Pratt parsing for expressions, recovery, `E1xxx` diagnostics. |
| `seal-ty` | Interned semantic types, nominal aggregates, structural tuples, arrays, slices, references, raw pointers, and trait objects; inference variables, unification, the `Copy`/`Sized`/`Drop` queries from the memory model, the shared initialization lattice, and the single classification of numeric and pointer cast pairs. |
| `seal-methods` | The closed compiler-owned intrinsic method catalog: owners, receiver modes, signatures, return kinds, unsafe gates, and semantic operations shared by type checking, MIR, and code generation, including the raw pointer methods. |
| `seal-layout` | Deterministic struct, enum, tuple, array, and vtable layout: field offsets, alignment, tag and payload layout, vtable header and method slots, and size derived from `TyCtxt` types. |
| `seal-def` | Definition identities: `DefId`, kind, name and spans, owner, and binding mutability. |
| `seal-hir` | AST lowering and name resolution: one body per function, trait default method, constant, or static, inherent impl and receiver registries, trait and impl coherence, default-method lookup, resolved paths, places, match patterns, and the control-flow and introspection built-ins, `E2xxx` diagnostics. |
| `seal-typeck` | Type and initialization checking: expression/local/signature types, generic argument inference and bounds, default trait method resolution, array and slice bounds, array-to-slice and trait-object coercion, object safety, address-space limits, explicit numeric casts, literal ranges, built-in divergence and type queries, mutability, match guards and exhaustiveness, `E3xxx` diagnostics. |
| `seal-mir` | Control-flow-graph lowering of checked HIR: locals, blocks, statements, field, tuple, index, variant, and raw pointer projections, instantiated aggregates, direct, symbolic, and trait-object call targets, trait-object construction and reborrow, numeric and pointer casts, raw intrinsic calls, bounds checks, match dispatch, type-information values, panic/exit terminators, and structural plus type validation. |
| `seal-borrowck` | Liveness, may-hold loan, and initialization dataflow over MIR: shared aliasing, moves, tuple and array element paths, cast operand reads, raw pointer origins, escaping references, global effects including raw accesses, `E5xxx` diagnostics. |
| `seal-mono` | Monomorphization: reachable-instance collection from non-generic function roots, specialization of MIR types, cast endpoints, and aggregate arguments, resolution of trait calls to impl methods or default bodies, vtable collection from code and constant values, drop glue collection per concrete type (destructor instance and children), reachable static and const-allocation collection, recursion, instance, vtable, and glue limits, stable instance and type fingerprints, and global validation (`E7xxx`). |
| `seal-codegen` | LLVM lowering of checked MIR, saturating numeric conversions, raw pointer loads/stores/arithmetic and copy intrinsics, one internal drop glue function per concrete type (vtable slot 0 points at it), vtable globals and indirect dispatch, DWARF emission, optimization, and object/assembly/executable artifacts with the embedded runtime (`E6xxx`). |
| `seal-llvm-ffi` | Isolated FFI bridge for GEP construction, `ptrtoint`/`inttoptr` observations, and the LLVM 19+ debug-record API that inkwell 0.11 exposes unsafely or mis-wraps; the only crate that permits `unsafe`. |
| `seal-version` | `LanguageVersion` and feature gates. |
| `seal-target` | Target triples and the properties derived from them. |
| `seal-fs` | UTF-8 source loading and atomic artifact writes. |
| `seal-profiling` | Phase timings for `--time-passes`. |
| `seal-query` | Revisioned incremental query database: named inputs, typed query keys, dependency validation by fingerprint, red-green backdating, statistics, cycle detection, and a versioned persistent byte cache. |
| `seal-session` | Owns options, target, source map, interner, diagnostics, gates, cancellation, profiler, and the session-scoped query database. |
| `seal-test-support` | Inline `//~` fixture annotations and structured matching. |
| `seal-stdlib` | Standard-library location, descriptor and package validation, stable digesting, prelude selection, and installation verification (`E8133`–`E8141`). |
| `seal-plan` | The versioned `CompilePlan` exchanged between the project layer and the driver: target, profile, packages, standard-library identity and modes, files, API paths, dependency aliases, cfg values, entry, and output. |
| `seal-manifest` | `seal.toml` parsing and validation: project/package settings, targets, path dependencies, env, profiles, API exposure, unknown-key warnings, and span-bearing diagnostics. |
| `seal-project` | Member and source discovery, path-dependency graph and cycles, standard-library mode selection, target selection, deterministic lockfile, metadata model, graph queries, affected-path mapping, and one plan per target. |
| `seal-api` | Versioned public-API snapshots, canonical hashing, and semver compatibility classification shared by the CLI and registry. |
| `seal-doc` | HTML and JSON documentation rendered from public API snapshots, plus a read-only documentation server. |
| `seal-cli` | The `seal` binary: templates, build/run/check, graph/affected queries, docs/API, metadata, doctor, tasks, and in-process driver invocation. |
| `seal-driver` | The `sealc` binary and driver library: clap-based subcommands and argument groups, single-file and plan-file pipelines, exit codes, panic policy, and phase wiring. |

## Compilation pipeline

The driver runs the phases in this order for every input. A phase runs only when
its predecessors reported no errors, so each phase may rely on the invariants
they establish.

```text
source
  -> lex          tokens
  -> parse        AST
  -> lower        resolved HIR
  -> typeck       checked HIR and TypeckResults
  -> lower        MIR bodies
  -> elaborate    drop sites expanded, guarded, and pruned
  -> const-eval   ConstProgram: values, allocations, relocations
  -> borrowck     loans, origins, and global effects
  -> mono         MonoProgram
  -> codegen      LLVM IR, object, or executable
```

| Phase | Crate | Consumes | Produces | Diagnostics |
|-------|-------|----------|----------|-------------|
| Lex | `seal-lexer` | source text | tokens and trivia | `E0xxx` |
| Parse | `seal-parser` | tokens | `seal_ast::Ast` | `E1xxx` |
| Lower | `seal-hir` | AST | resolved HIR and bodies | `E2xxx` |
| Type check | `seal-typeck` | HIR | types, coercions, checked places | `E3xxx` |
| Lower MIR | `seal-mir` | checked HIR | control-flow bodies | internal validation |
| Drop elaboration | `seal-mir` | MIR | guarded and pruned drop sites | internal validation |
| Const evaluation | `seal-const-eval` | MIR | `ConstProgram` | `E4xxx` |
| Borrow check | `seal-borrowck` | MIR and `ConstProgram` | loans, origins, effects | `E5xxx` |
| Monomorphize | `seal-mono` | MIR, `ConstProgram` | `MonoProgram` | `E7xxx` |
| Code generation | `seal-codegen` | `MonoProgram`, `ConstProgram` | IR and artifacts | `E6xxx` |

The project layer sits in front of this pipeline. `seal-manifest` parses and
validates `seal.toml`; `seal-project` discovers members and sources, resolves
path dependencies, detects cycles, builds targets, and writes `seal.lock`;
`seal-plan` serializes the resulting graph. `seal` writes a plan and invokes
the driver in-process, while `sealc --plan-file` loads the same plan directly.
The driver validates every plan invariant before loading a source file, so a
malformed graph is `E8001`/`E8010` rather than a partial compilation.

Workspace queries are derived from the same loaded model. `seal graph`
produces sorted package, target, action, and module projections; `affected`
maps a VCS diff back through package roots, target files, generators, the
lockfile, and the toolchain, then follows reverse dependencies. The project
architecture and phase-10 contracts are documented in
[`project/architecture.md`](project/architecture.md).

HIR lowering validates trait-impl coherence first, then opt-in `Copy`
implementations, then `Drop` implementations. `Copy` validation registers
candidates and removes those whose fields fail until the registry reaches a
fixed point, so declaration order does not change the result.

`seal-mir` elaborates drop sites between MIR lowering and constant evaluation:
it expands one site into the values its glue visits, removes the sites whose
value is definitely gone, guards the path-dependent ones with a `bool` flag, and
drops the value an assignment replaces. The order matters. A constant evaluated
with a dead drop site would be rejected for a destructor it never runs
(`E4028`), so the dead sites are gone before evaluation starts.

`seal-const-eval` is introduced together with the global-data feature and sits
between MIR lowering and borrow checking, so later phases consume evaluated
values instead of re-evaluating source. Its modules are `value`, `allocation`,
`program`, `interpreter`, `place`, `rvalue`, `operators`, `intrinsic`,
`dependency`, `limits`, and `error`, each with tests beside it.

### Modules, imports, visibility, and configuration

The parser accepts the module surface before lowering learns to resolve it. A
file may declare one namespace with `module path::to::module;`, followed by
absolute imports and items:

```text
module? use* item*
use geometry::{Circle, Polygon as Poly};
use root::geometry::shapes::Circle;
```

`module`, `use`, `public`, `internal`, `private`, `cfg`, and `test` are
contextual names. They are recognized only in the modifier prefix of an item,
field, method, or import, so existing bindings, functions, and types spelled
with those names keep parsing unchanged. Visibility is a property of items,
fields, inherent methods, and re-exports; a plain import is file-scoped. Variants
inherit their enum's visibility, and trait methods inherit their trait's. A
module is always internal to its package and cannot take a modifier.

`cfg(...)` is a typed boolean expression over `feature("name")`, `env("NAME")`,
`target.{os,arch,env,pointer_width}`, `debug_assertions`, `test`, and `bench`,
with comparisons, `!`, `&&`, `||`, and parentheses. The AST stores the
expression in its own arena instead of a raw string, so a misspelled key is
diagnosed at parse time rather than silently evaluating to false. `cfg` selects
items and imports, not fields or variants, because the latter are part of their
owner's layout.

Expression and type paths with two or more `::` segments keep every segment in
`PathMulti`, `StructMulti`, and `TypeKind::PathMulti`; the existing
`Type::associated` form with one segment keeps its shape. `cfg` conditions are
parsed and retained on every item and import, but lowering does not select
configuration yet: the project layer supplies features and target options in a
later phase, and until then every retained declaration is analyzed.

### Multi-file packages

`seal_hir::lower_package` lowers every file of a package against one
`PackagePlan`. The plan lists packages in dependency order, the files of each
package, the modules exposed as API, and dependency aliases. The driver builds
the plan for `sealc --plan` and for a single file, so both pipelines share the
same package and standard-library injection path.

`ModuleGraph` is built before any body is lowered. Each file names its module
with at most one `module path::to::module;` declaration as the first item; a
file without one contributes to the package root. Parent modules are implicit,
so `module app::util;` creates `app` when no file declares it. Several files may
declare the same module and their items share its namespace; a duplicate name
across fragments is `E2083` with both spans, and a late, repeated, or empty
declaration is `E2069`.

Imports resolve before lowering bodies. A `use` path is absolute: it starts at
the package root (`root`), at a top-level module, or at a dependency alias,
followed by module segments and one item or module. Groups expand into one
binding per name, `as` renames the local binding, and globs are rejected
(`E2076`). The bindings live in a per-file `ImportTable`, so two fragments of
one module never share the names they imported. The graph resolves each leaf
against its `ModuleGraph`: an unknown module is `E2070`, an unknown item is
`E2073`, a name that denotes both a module and an item is `E2074`, a local name
that collides with another import is `E2078`, and one that collides with an item
or submodule is `E2077`. Importing a name that resolution never uses emits the
`unused_import` warning.

`public use` and `internal use` are re-exports, resolved to a fixed point before
any plain import. A re-export is an alias, not a declaration: the graph stores
the canonical target module or item, and import, qualified-path, and API
resolution all share that target. `public use` requires the declaring module to
be exposed, the target item to be public, and every module on the target path to
be exposed; `internal use` may target `internal` items inside the package, while
a target from another package must still be public and exposed. Re-export
chains and diamonds are valid; a cycle is `E2089`, a duplicate name is `E2090`,
a target that cannot be aliased is `E2091`, a visibility widening is `E2092`, an
unreachable target module or internal hop is `E2093`, and a collision with an
item or submodule is `E2094`.

Each package entry may also name a prelude source: either a user package index
or a standard-library package name, plus a module path inside that package.
After re-exports and deferred exposures settle, the
graph collects the public item exports of that module into a `PreludeTable`.
Only items and public re-export aliases enter; a module export is `E2101`, a
missing module is `E2100`, a same-namespace collision is `E2102`, and a cycle
between package preludes is `E2103`. Unqualified lookup then resolves local
scopes first, module items second, explicit file imports third, and the prelude
last, independently in each namespace. Prelude names are not import bindings,
so they never produce `unused_import`; an explicit import that shadows one still
warns when unused. A package without a prelude uses no implicit names, and
`core`/`std` plans leave the field absent so the prelude cannot prelude itself.
The source is plan data, not a hardcoded list, and there is no glob expansion.

### Standard library

A package selects `std`, `core`, or `none` in its manifest. The project layer
records that mode per package and puts the selected library's language, version,
and digest—never a path—in `CompilePlan` version 2. The driver locates the local
installation (`SEAL_STDLIB`, then the sysroot, then the development tree),
validates it, and injects the library packages before the user packages. User
package indexes and aliases are preserved; the HIR sees the standard library as
ordinary packages with reserved `core`/`std` aliases. The digest participates in
the frontend action key and every library source is an explicit build input, so
an edited standard library invalidates cached work while a touched timestamp
does not. Diagnostics use `<stdlib>/...` identities and never expose the
installation path. Modes without a library (`stdlib = "none"` and no prelude)
require no installation. See [`project/stdlib.md`](project/stdlib.md).

Inside a body, a multi-segment path resolves its prefix through the module graph
and its last segment through the value, type, or trait namespace, after the
file's imports. `root::name` and `module::name` work in expressions and types,
including an impl's self type; `Type::associated` keeps its existing meaning. A
module used as a
value is `E2086`, and a module that cannot be reached from another package is
`E2071`.

Every definition carries its package, compiled instance, module, visibility,
optional documentation, and source file in `DefData`. Visibility is enforced at
each use and import: `private` reaches the declaring module and its descendants,
`internal` reaches the package, and `public` additionally requires the declaring
module to be exposed. Exposing a module also exposes every non-root ancestor
needed to reach it; the root is exposed only by listing `root` explicitly. An
exposure path may end in a module re-export; its prefix is exposed before
exports are resolved and the alias leaf is resolved afterwards, so facades such
as `std::option` can be published while the canonical target module is also in
the API. A `public` item in a module outside the API is `E2080`; an exposure
path that resolves to neither a canonical module nor a public module re-export
is `E2081`. Signatures of public functions, values,
trait methods, fields, and variants are validated after bodies are lowered, so a
public signature that names a non-public type is `E3100`–`E3105`. An impl may
only live in the package that declares the type or the trait (`E2082`).

`Hir` exposes the resulting structure through `module_of_def`,
`package_of_def`, `items_in_module`, `is_visible_from`, `public_items`,
`is_public_api`, and `primary_package`. Symbols and instance fingerprints
include the package instance and, when the compilation has more than one
package or the definition is not in the root module, the module path, so equal
names in different packages or modules never collide.

### Callable values

A Seal function pointer is `TyKind::FnPtr(FnPtrId)`. `FnPtrDef` stores the
ordered parameter types, result, and `is_unsafe`; the type is `Sized`, `Copy`,
and one word, and it never carries a C ABI or a variadic tail. A function item
in a value position has the pointer type of its signature, so lowering, MIR,
and monomorphization use the existing call path instead of a second callable
representation.

A closure has two linked HIR products: a `BodyKind::Closure` body whose first
parameter is the environment, and an `AdtKind::Closure` environment with one
field per capture in first-mention order. Each capture records whether it comes
from the creating frame or an enclosing closure. Type checking records the
lifted signature, the capture storage (`Shared`, `Exclusive`, or `Value`), and
the closure kind. MIR lowers a closure call to `CallTarget::Closure`, a
function-trait call to `CallTarget::FnTrait`, and a function pointer value to
the matching `FnPtr` type.

Monomorphization substitutes the closure's arguments, resolves a symbolic
`FnTrait` target to the concrete closure or pointer, and keeps the receiver
shape the bound requires: a borrow for `Fn`/`FnMut`, the value for `FnOnce`.
A pointer receiver introduced as a borrow is dereferenced back to the pointer
value. The closure body keeps its environment parameter, so a weak-bound call
adapts it at code generation: `*Env` and `*mut Env` share a representation,
and a by-value `Env` is materialized in a local slot so an `Fn`/`FnMut` body can
borrow it.

Code generation emits one internal function per closure instance, named from
the parent and stable instance hash, with `<closure>` debug information and
one formal parameter per capture. A capture-free closure converted to `func`
uses one internal trampoline per `(closure instance, pointer type)`; the
trampoline allocates the empty environment, calls the lifted body, and returns
its result. A C callback still uses the separate C-ABI trampoline. Function
pointers and closures never allocate, and no callable value introduces a
vtable, heap object, or runtime lookup.

## Invariants

1. **No global mutable state.** Two sessions in one process are independent.
2. **Spans never cross files.** `SourceMap::span` validates ordering, bounds, and
   UTF-8 character boundaries. `file_of` rejects ranges covering the one-byte gap
   between files.
3. **One place counts lines.** `LineIndex` owns line breaks and line/column
   conversion; columns are one-based Unicode scalar values, with separate UTF-16
   columns for LSP.
4. **BOM is removed once.** `SourceMap::add_file` strips a leading BOM and records
   it; all offsets are relative to the stripped text.
5. **Errors are values, not strings.** Emitting an error returns
   `ErrorGuaranteed`; `Session::abort_if_errors` refuses to continue without it.
6. **Diagnostics are deterministic.** Emitters do not read the clock, environment,
   or terminal width implicitly; color is resolved by the caller and injected.
7. **Persisted hashes are stable.** Only `seal-hash`'s `StableHasher` may be used
   for cache keys or metadata. `FxHash*` is for in-memory tables only. Changing
   the stable hash requires bumping `FORMAT_VERSION`.
8. **Symbols are process-local.** The interner leaks strings until process exit and
   never serializes a `Symbol`; persist `resolve(symbol)` instead.
9. **Codes never change meaning.** New diagnostics get new codes in their phase's
   range; warnings are identified by lint name, not code.
10. **Writes are atomic.** Artifacts are written to a temporary file and renamed;
    a failed build never leaves a truncated output.
11. **Tokens are ordered and total.** The lexer always ends with an `Eof` token
    whose span is the empty range at the end of the file; token spans never
    overlap, never leave the file, and every malformed lexeme becomes one
    `Error` token covering it, so parsing can resume at a known boundary.
12. **Lexing is context-free and total.** The same text always produces the same
    tokens, independent of target or phase; the scanner always advances, and
    non-user states are reported as ICEs rather than loops. Comments are trivia
    (including nested block comments and the four doc-comment styles), and a
    newline inside any comment still sets `preceded_by_newline`.
13. **Source text is not a Trojan Source vector.** Bidirectional formatting
    characters are rejected everywhere, raw control characters are rejected in
    literals, non-ASCII whitespace and bare carriage returns are rejected, and a
    BOM is only accepted at the start of a file.
14. **The AST is total.** Parsing always returns a tree with a program, even for
    hostile or truncated input: invalid syntax becomes an `Error` item, statement,
    expression, or type carrying the offending span, so later phases and editors
    can traverse the tree without special-casing missing nodes.
15. **The AST owns its nodes.** Nodes live in dense arenas and refer to each
    other only through typed, non-zero IDs. Every node is reachable from the
    program, every ID resolves, and the parser is the only writer, through
    `AstBuilder`; consumers never mutate the tree.
16. **Spans are exact and nested.** Every node span lies inside the file, a
    parent's span covers its children, and `seal_ast` keeps the span of each name
    and literal so later phases can point at the original text instead of rescanning.
17. **Types are canonical.** Every type is interned, so two types are equal
    exactly when their IDs are equal; the arena never holds two equal kinds, and
    comparing two types never walks the structure.
18. **Unification is sound.** Only inference variables are bound, every binding
    passes an occurs check, and distinct scalar types never unify. The only
    implicit conversion is trait-object pointer unsizing, selected at an
    expected-type boundary and recorded on the expression. `Error` is the only
    type that absorbs another, and it is only built after a diagnostic was
    reported.
19. **Memory classification is derived.** `Sized` and `Drop` are queries over
    the interned type, never flags stored beside it. `Copy` is structural for
    built-in types and backed by the validated `CopyImpls` registry for nominal
    ADTs; the registry is derived from the declaration and every field, so the
    answer cannot contradict the type or the memory model.
20. **Bindings are immutable unless declared `mut`.** The lexer reserves `mut`,
    the AST records the flag per binding, and no later phase infers mutability
    from how a name is used.
21. **Parsing terminates and recovers.** Every loop asserts progress, recursion
    is bounded by `MAX_DEPTH`, and recovery synchronizes at `;`, `}`, `func`, and
    end of file. The same token stream always produces the same tree, and a
    malformed construct costs one error rather than a cascade.
22. **Control flow is structured.** `if` and `if let` are expressions with
    block branches, `else if` is a block whose tail is another `if` or `if let`,
    and `while`, `while let`, and `for` are statements with a block body.
    `break`/`continue` target the innermost loop. A block-like expression used
    as a statement needs no semicolon.
23. **Every HIR name is resolved.** A `Path` holds a `Res` (a `DefId`), never a
    `Symbol`; a name that does not resolve becomes an `Error` node after `E2002`
    was reported, so no consumer performs a textual lookup.
24. **HIR is total and owner-partitioned.** Every function owns exactly one
    body, every ID resolves, every node is reachable from its body, and every
    parameter or binding definition records the function that owns it. A body
    is never shared or orphaned. A closure is owner-partitioned the same way:
    it owns a `BodyKind::Closure` body and a synthetic `AdtKind::Closure`
    aggregate whose fields are its captures, and each capture records its
    final `origin` plus whether the environment reads it from the creating
    body (`CaptureSource::Local`) or from the immediately enclosing closure
    (`CaptureSource::Outer`). A name from an enclosing body never resolves to
    `Res::Local` inside a closure: it becomes `Res::Capture`.
25. **Loop control is checked.** `break` and `continue` may appear only inside a
    loop. Lowering tracks loop depth, reports `E2003`/`E2004` outside one, and
    keeps the statement so later phases still see the shape that was written.
26. **Scopes follow the source.** Resolution walks ribs from the innermost
    outwards; a `let` inserts after its initializer, so a binding is not visible
    in its own initializer, and `if`/`while`/`for` bodies never leak bindings;
    a `for` pattern is scoped to its body, an `if let` pattern is scoped to its
    then block and never its else branch, and a `while let` pattern is scoped to
    the loop body.
27. **Types are assigned under one coercion rule.** Checked types are exactly
    equal, or one side is `!`/`Error`, or a thin pointer to a sized concrete
    value coerces to a trait-object pointer of the same reference kind, or a
    function item coerces to a Seal `func` pointer with the same signature. There
    is no implicit numeric conversion (D12). A literal adopts the expected type
    only when that type belongs to the same numeric family, and a suffix always
    wins.
28. **Every value expression has a type.** The results map covers every HIR
    expression except the callee of a valid direct call, which is not read as a
    value. Every local has a type and every function has a signature. A function
    item used in a value position has the `func` pointer type its signature
    names.
29. **Plain conditions are boolean.** `if` and `while` conditions are checked
    against `bool`; there is no truthiness. `if let` and `while let` replace the
    condition with a pattern match instead.
30. **Initialization is definite.** A local read requires initialization on
    every path that reaches it. Assignments in both `if` branches count only at
    the join; a branch that diverges does not participate; a `while` or `for`
    restores the state from before the loop. An uninitialized immutable binding may be
    initialized once by assignment; reassigning it needs `let mut`.
31. **Stored values are sized.** Locals, parameters, and return values cannot
    have `str` or `[T]`; a slice is only usable through `*[T]`/`*mut [T]`, an
    array is sized when its element is, and its total size must fit the target
    address space (`E3023`). An array reference coerces to a slice reference when
    the expected type requires one. Literals must fit the integer or float type
    selected for them, using the target pointer width for `isize` and `usize`.
32. **MIR is a control-flow graph.** Every body has one entry block and a return
    place at local zero; parameters are locals `1..=arg_count`, bindings and
    temporaries follow, and every local declares its type. Every block ends in
    exactly one terminator, so there is no implicit fallthrough, and validation
    rejects a body whose place or edge names a missing local or block.
33. **Evaluation order is preserved.** Operands are lowered left to right before
    the operation that consumes them, and a simple assignment is the one
    exception: its value is lowered before the target place, so
    `grid[0] = grid[0] + 1` can read the container before the target borrows it.
    Compound assignments lower the target place once before the value, because
    they read the old value through that place. Statements keep source order,
    and a call is a statement (`Rvalue::Call`) because Seal has no unwinding, so
    a call has exactly one successor.
34. **Short-circuiting is control flow.** `&&` and `||` lower to `SwitchInt`
    graphs whose short-circuit block stores the constant result, so the right
    operand is evaluated only on the path that needs it. MIR has no `BinOp` for
    them.
35. **Loops and branches are explicit.** `if`, `if let`, `while`, `while let`,
    `for`, `break`, and `continue` lower to blocks and edges; boolean conditions
    use `SwitchInt` with `1` for true, an `if let` pattern jumps to its else or
    unit path on failure, and a `while let` pattern jumps to the loop exit on
    failure. `while` continues to the condition block, `while let` continues to
    the pattern test so the scrutinee is reevaluated, `for` continues to the
    increment block so an index or counter always advances, and `break` jumps to
    the loop exit. A condition is always evaluated in its own block, once per
    test.
36. **Lowering assumes a clean check.** MIR is produced only from a HIR tree
    that type checking accepted without errors, so every value expression has a
    type, every assignment target is a local place, and every local has a
    storage type. The driver validates the lowered tree and reports a structural
    failure as an internal compiler error.
37. **Divergence does not truncate the graph.** A `return` terminates its block,
    but lowering keeps processing the rest of the source, so unreachable blocks
    may exist and must stay structurally valid. Validation does not require
    reachability; removing dead blocks belongs to a later pass.
38. **References are checked, never annotated.** `&place` creates a `*T` loan and
    `&mut place` creates a `*mut T` loan; `*reference` reads through either, and
    `RefKind` is part of the type, so the two never unify. A dereference of a
    reference value is a place as well, so `&*identity(p)` borrows the pointee
    and lowering keeps the reference itself in a temporary. Regions are inferred
    from liveness; there is no lifetime syntax, and no region appears in a type.
39. **Loans conflict by kind.** `*T` is read-only, so a shared loan conflicts
    only with assigning or moving the borrowed place; two shared loans may
    overlap, and reads never conflict (`E5001`/`E5002`). A `*mut T` loan is
    exclusive: it conflicts with every other loan of the same place and with
    reads, writes, and moves of it (`E5007`/`E5008`). A loan or access reached
    through a reference is compared with the storage the reference may name, so
    copies and reborrows are ordered; a reborrow records the loans that
    authorized it and does not conflict with them. A conflict labels the live
    borrow's last active use, naming where the access would have to move.
    `&mut place` requires a writable place (`E3017`), and reading a non-`Copy`
    pointee by value is `E3018`. A method receiver is a place as well: it is
    typed without reading its value, an expected reference is satisfied by
    borrowing it, and `E3018` applies only when the call passes a non-`Copy`
    receiver by value.
40. **Loans are tracked per point, not per region union.** A local's may-hold
    set is a forward dataflow fact: overwriting a reference drops the loans it
    carried, and a loan is active only while a live local may hold it. This
    accepts programs where a place becomes writable again after the last alias
    dies, including inside loops. A parallel may-alias analysis with the same
    shape records the storage each reference path may name; conflicts compare
    those storages, and a reference read out of memory aliases conservatively.
41. **A reference never outlives its storage.** A returned loan of a whole local
    is `E5004`; a loan through a reference (`&*parameter`) borrows data outside
    the frame and is accepted. The loans that the reference itself carries are
    checked too, so `&*identity(&local)` stays `E5004`: the pointee belongs to
    this frame even though the returned place is reached through a reference.
42. **Initialization is rechecked on places.** The checker tracks
    uninitialized, initialized, and moved for every local on the MIR graph. A
    read of a moved place is `E5005`, and a read that is not initialized on
    every path is `E5003`.
43. **Borrow checking assumes a clean type check.** The driver runs it only
    after type checking reported no errors, so every place, operand, and edge is
    well formed; `seal-mir` validation runs first, and a structural failure is
    an internal compiler error rather than a user diagnostic.
44. **Nominal aggregates and structural tuples are separate kinds.** A struct or
    enum declaration interns an `AdtId`, so two aggregates with identical
    members are different types. A tuple interns its ordered element types in a
    structural `TupleId`, so equal spellings denote the same type; `()` remains
    `TyKind::Unit` and is not a distinct empty tuple. `Copy`, `Sized`, `Drop`,
    and `contains_reference` derive from struct fields, variant fields, tuple
    elements, and array elements, with a cycle guard so a malformed declaration
    cannot hang a query.
45. **Struct and variant literals are complete.** Every field of the chosen
    struct or variant appears exactly once, every named field exists, every
    initializer has the declared field type, and a literal in value position is
    rejected in favor of a type name (`E2005`–`E2010`). A variant is unit,
    positional (`Some(i32)`), or named (`Circle { radius: i32 }`). Positional
    arguments bind in declaration order, so names and parentheses cannot be
    mixed (`E2036`–`E2039`). Duplicate aggregate, variant, and field names are
    rejected at declaration, and enum-specific mistakes use `E2029`–`E2039`.
46. **Fields are places, not accessors.** `p.x` and `pair.0` denote storage:
    each needs a place base (`E2012`), never autodereferences a `*T` (`E3015`
    tells the user to write `(*p).x` or `(*pair).0`), and records the selected
    field's `DefId` or tuple position once for MIR. Reading one field or element
    reads only that move path; it does not require sibling fields to be
    initialized. An enum value
    has no field place until `match` proves its variant: the `VariantField`
    projection is valid only after the discriminant test selected it.
47. **Aggregates own their fields.** A struct or enum value moves as a whole
    unless it has an explicit `impl Copy` covering the declaration and every
    field of every variant; a tuple is `Copy` exactly when every element is. A
    `Copy` field or element still copies when read. Field writes
    require the root binding to be `mut` (`E3004`), because a field write does
    not initialize the binding as a whole. Field-by-field initialization needs
    a declared type and `mut`, and a whole assignment initializes every field at
    once.
48. **Recursion needs indirection.** A struct or enum cannot contain itself by
    value (`E3016`); a reference field breaks the cycle. Every struct field and
    every enum variant field must be `Sized` (`E3010`).
49. **Layout is deterministic.** `seal-layout` assigns struct fields and tuple
    elements offsets in declaration order with natural alignment and no
    reordering, so the same types produce the same layout in every phase and on
    every target with the same data model.
    An enum adds a tag sized by its variant count and a payload at the widest
    variant alignment; variant offsets are absolute.
50. **MIR places carry projections.** A `Place` is a local plus
    `Projection::{Deref, Field(DefId), TupleField(u32), Index, VariantField}`; a
    struct literal lowers to `Rvalue::Aggregate` with `AggregateKind::Struct`, a
    variant literal lowers with `AggregateKind::Variant { adt, variant }`, a
    tuple literal lowers to `Rvalue::Tuple`, and a non-`Copy` field or tuple
    element read is a `Move`. Validation checks every projected place's local. A
    dereference of a reference value (`*identity(&v)`, `*&x`) has no local to
    project from, so lowering stores the reference in a temporary and projects
    through it; the read still copies, because the place is reached through a
    reference.
51. **Move paths reach into aggregates.** Type checking and borrow checking
    share `seal_ty::InitPaths`: a field or tuple-element assignment initializes
    only that path, reading an uninitialized field is `E3011`/`E5003`, and a
    read of a field whose sibling moved is allowed. Borrow conflicts compare path prefixes, so
    borrowing `p.x` leaves `p.y` writable and borrowing `p` blocks both. A
    whole-place borrow with a moved field is `E5006`.
52. **Code generation consumes only monomorphized MIR.** The driver calls the
    backend only after type checking, borrow checking, and monomorphization
    report no errors, and both MIR and `MonoProgram` validation succeed first.
    The backend iterates `MonoProgram` instances, never generic bodies, so a
    layout failure, a leftover type parameter or inference type, or invalid
    generated IR is an internal compiler error, never an artifact.
53. **Every local has storage.** Locals, parameters, and the return place are
    `alloca` slots; reads and writes are loads and stores, and calls use the
    checked signature. Aggregates are built with `insertvalue` in declaration
    order, struct fields and tuple elements are projected with struct GEPs, and
    arrays and slices with a bounds-checked inbounds element GEP, so evaluation
    order, layout, and pointer provenance agree.
    A slice range is lowered to explicit checks followed by a fat-pointer
    construction; `len` reads the length member.
54. **The ABI is compiler-owned and per instance.** A non-generic free function
    uses the `seal.<name>` symbol and a non-generic impl function uses
    `seal.<Type>.<name>`, so same-named methods on different types cannot
    collide. A generic instance appends the stable fingerprint of its full type
    arguments, so two instantiations never share a symbol and repeated builds
    agree. Generic aggregates are one LLVM struct per `(definition,
    arguments)` pair. The C `main` wrapper is the only `main`, and only a
    non-generic free function can be the language entry point. A user `main`
    takes no parameters and returns `i32` or `()`. `*str`, `*[T]`, and
    `*mut [T]` are `{ data, length }`; `*dyn Trait` and `*mut dyn Trait` are
    `{ data, vtable }`; `()`/`!` are zero-sized structs; the object is position
    independent.
55. **Debug information describes the generated IR.** `-g` emits a Rust compile
    unit, one subprogram and one aggregate type record per instance with
    readable type arguments, parameters, bindings, aggregate types with layout
    offsets, and a source location per statement and terminator, all finalized
    before verification and optimization.
56. **Arithmetic cannot silently invoke LLVM undefined behavior.** Integer
    division and remainder panic through the runtime on a zero divisor and on
    the signed minimum divided by `-1`, reporting the failing expression's
    source location and exiting with status 101. The `panic(message)` built-in
    uses the same reporter with a computed `*str`, and `exit(code)` uses the
    runtime's direct process-exit primitive; both are terminators with no
    successors. Shift amounts are masked to the operand width.
    Integer arithmetic wraps, floats follow IEEE, and no overflow check is
    inserted.
57. **Artifacts are all-or-nothing.** Object and assembly output are written
    atomically. Executables are linked into a temporary path from a temporary
    object and the embedded runtime archive, and replace the destination only
    after a successful link. Every temporary file is removed on every path, and
    the linker's stderr is part of the diagnostic.
58. **Targets are explicit.** The LLVM triple and target machine come from
    `seal-target`, never from the host. An unavailable backend is `E6002`, and a
    missing linker, unsupported cross-link, unreadable external runtime, or
    runtime/target mismatch without `--runtime-archive` is `E6003`. Linker
    arguments adapt to `cl`/`clang-cl`, `link`, and GNU-style toolchains.
59. **The runtime is layered and self-contained.** The embedded C11 runtime has
    an allocation-free core (I/O, panic, and exit) and a separate heap layer.
    The core takes no locks and uses no stdio, locale, or `atexit` handlers;
    writes transfer exactly the requested bytes, retrying `EINTR` and partial
    writes; panics never unwind and always terminate with status 101; `exit`
    terminates with the requested status. The heap exposes
    `seal_alloc`, `seal_alloc_zeroed`, `seal_realloc`, and `seal_dealloc`.
    Allocation and reallocation validate alignment, return `NULL` on invalid
    layout, overflow, or allocation failure, and treat a zero size as a
    dangling aligned pointer; reallocation preserves the prefix, keeps the
    alignment, and leaves the old block valid when it fails. Deallocation is a
    no-op for a null pointer or a zero size. The heap never panics. The archive
    is host-specific and linked only into executables, so object and assembly
    output stay target-independent; unused members, including the heap, are not
    linked. `sealc emit-runtime` exports the embedded archive, and
    `sealc build --runtime-archive <PATH>` supplies a target archive.
60. **The command line is declarative.** `seal-driver` parses a required
    subcommand with `clap`. Each command declares only the options it honors:
    shared compile options (inputs, target, diagnostics, optimization, runtime)
    are reused, while output selection belongs to `build`. Checking is a
    command, not an artifact kind, so it cannot write files. `run` owns a
    temporary executable path, executes only host targets, forwards arguments
    after `--`, and returns the child status. One module converts the parsed
    arguments into session options, and rules that span arguments, such as a
    single `-o`, are ordinary usage errors with status 2.
61. **A call accesses through its reference arguments.** An exclusive argument
    (`*mut T`, or an aggregate carrying one) gives the callee sole access to its
    pointee for the whole call, so the checker rejects it when a live loan of
    that storage remains (`E5009`). Two reference arguments may share storage
    only when neither is exclusive; otherwise the call is `E5010`. Both rules
    compare the storage the may-name analysis records, so copies and reborrows
    are ordered. An argument whose target is unknown cannot be proven, so the
    checker stays silent there and the callee loses `noalias` instead.
62. **`noalias` is a proven fact, not a language guarantee.** Borrow checking
    returns the functions for which every exclusive-reference parameter can be
    marked `noalias`: the body never reaches storage through a reference of
    unknown target or a raw-memory operation, and every call site proves its
    reference arguments are known and disjoint. A function outside the set
    keeps the ordinary ABI and
    remains correct; the attribute never decides whether a program is accepted.
    A shared `*T` parameter is always `readonly`, because the type system
    forbids writing through it. A function reachable through a vtable is always
    outside the set, because indirect call sites never passed the whole-program
    proof.
63. **Method dispatch resolves through per-type, catalog, and object lookup.**
    A method call is a dedicated AST and HIR node, not a desugared field access.
    Type checking first looks up `(struct, name)` in the inherent impl registry
    and then, for non-struct receivers, the intrinsic catalog after following
    the receiver's reference layers. It validates the signature and writability
    and records whether the call is a user function, a trait function, an
    intrinsic, or a dynamic slot, the receiver adjustment, dereference count,
    and result type. A static user or trait method lowers to a direct
    `Rvalue::Call` whose first argument is the receiver, with an explicit
    `Rvalue::Ref` when the receiver is borrowed automatically; an intrinsic
    lowers to `Rvalue::Intrinsic`. A dynamic call lowers to
    `Rvalue::DynamicCall`, which carries the principal trait reference, selected
    slot, receiver value, and argument types; the receiver is reborrowed as the
    method declares, and the borrow checker sees the same loan as a written
    borrow. Code generation consumes the recorded form and never re-infers
    ownership or the receiver shape. The lookups are ordered, so a struct method
    shadows an intrinsic with the same name. Trait methods add one layer to the
    nominal lookup: the inherent registry wins, then the functions bound by the
    type's `impl Trait` blocks, and a name two implemented traits declare is
    ambiguous (`E2028`). An omitting impl contributes the trait's default body
    as its candidate, so the same lookup selects either an override or the
    default before monomorphization. Each implementation is validated as a contract
    (`E2021`–`E2026`) with `Self` substituted by the implementing type, and it
    lives beside the inherent registry rather than in it, so an inherent method
    and a trait method may share a name. A `*mut T` receiver passed to a shared
    method is reborrowed as `*T`, lowering to a shared `Rvalue::Ref` of the
    pointee.
64. **`match` evaluates its scrutinee once and tests it structurally.** Lowering
    moves the scrutinee into a temporary, reads the tag with
    `Rvalue::Discriminant`, and dispatches with `SwitchInt`; a literal pattern
    compares with `BinOp::Eq`, a range pattern compares with `Ge` and `Le`/`Lt`,
    and struct, tuple, and array patterns project fields, positions, and
    constant indices without
    testing anything else. Or-patterns expand into one test chain per
    alternative, bindings are stored only after the whole pattern matched, and
    `Copy` bindings are stored before a guard while non-`Copy` ones follow it
    (`E2045`). Coverage and reachability come from the usefulness algorithm
    (`E3032`, `E3033`), and a guard never contributes coverage. Scalar ranges
    are constructors over domain partitions, so adjacent integer or character
    ranges are exhaustive and gaps have concrete witnesses. A slice pattern
    tests the runtime length and then indexes the dereferenced slice; coverage
    partitions the length domain at every pattern boundary and reuses the array
    usefulness check per length, so exact, prefix, and suffix patterns can
    exhaust a slice. Struct and variant patterns follow references by
    dereferencing the scrutinee place before the tag read or field projection;
    coverage unwraps the same references, so exhaustive reasoning sees the
    aggregate. `if let` reuses the same pattern tests for one refutable pattern;
    failure jumps to the else block, or to a unit value when there is no else,
    while success binds the pattern before lowering the then block. `while let`
    performs the same tests at the loop head, binds on success, and adds a
    backedge from the body to the head; failure exits the loop.
65. **Trait objects are unsized and object safety is checked when they form.**
    `dyn Trait` is a `TyKind` whose pointee has no layout; `*dyn Trait` and
    `*mut dyn Trait` are two-word fat pointers. Forming the type runs object
    safety over its principal trait and every supertrait: only `*Self`/`*mut
    Self` receivers, no `Self` elsewhere in a signature, no generic methods, and
    no associated functions (`E2065`). A trait remains usable statically.
    Coercion is selected only at an expected-type boundary, preserves the
    reference kind, requires an implemented trait (`E3046`), and is recorded on
    the expression as `Coercion::TraitObject`. A coerced dyn pointer reborrows
    as `Coercion::TraitObjectReborrow` with the same loan.
66. **Vtables have one deterministic key and slot order.** Monomorphization
    collects one vtable per reachable `(concrete type, fully substituted trait
    reference)` pair and one concrete method instance per slot. The key contains
    no `Param` or `Infer`. Slot order is the shared `seal-layout` order: drop,
    size, align, then principal-trait methods in declaration order and
    supertraits depth-first, deduplicated by method identity. Validation rejects
    a null slot, a method from a different trait, and a call whose principal
    reference differs from the receiver's pointee.
67. **Vtable construction and dispatch are explicit in MIR and IR.**
    `MakeTraitObject` copies the data address and the table address without
    copying the value, and `TraitObjectReborrow` copies the pair without a new
    loan. `DynamicCall` loads the recorded slot from the receiver's own vtable
    and emits an indirect call; code generation never looks up a vtable from the
    concrete type at runtime. The drop slot is the concrete type's own drop
    glue: monomorphization collects one glue per vtable type, and the backend
    emits it as an internal `void (ptr)` function, empty when the type declares
    no destructor, so the slot is never null or shared. Every vtable-reachable
    method is excluded from `noalias`.
68. **Numeric casts have one classification from source to machine code.**
    `seal-ty`'s `NumericCastKind` and `TyCtxt::numeric_cast_kind` decide which
    conversion a pair of concrete numeric types performs. Type checking records
    the kind once, MIR carries it with both endpoint types, monomorphization
    substitutes the endpoints and rejects a surviving `Param` or `Infer`, and
    code generation switches on the kind instead of re-classifying the types.
    No phase introduces a second numeric-pair table or an implicit conversion.
69. **Default trait methods are bodies with a symbolic fallback.** A trait
    method declaration may own a body; HIR lowers it with `Self` as a parameter
    and records a `ParamEnv` bound proving `Self` implements the declaring
    trait, without adding `Self` to the trait's `Generics` or changing coherence
    or object safety. An impl that omits the name stores the trait reference in
    the lookup registry, and a call on a concrete type records the default body
    with a fully substituted `Instance`. Calls inside a default body stay
    symbolic, and monomorphization selects an override when the implementation
    supplies one or the default instance otherwise. Associated functions without
    a receiver follow the same fallback, including `Self::name()` inside a
    default body. Vtable slots use the same resolver, so an omitted method points
    at the default instance; layout and MIR shapes do not change.
70. **Operator protocols use ordinary calls, not new MIR forms.** HIR installs
    `IntoIterator`, `Iterator`, `Index`, and `IndexMut` before user item
    collection, reserves their names (`E2066`), and rejects impls for
    compiler-owned sequences (`E2067`). Type checking records the selected
    protocol call and the iterator cursor in `TypeckResults`; lowering emits the
    existing `Rvalue::Call` or `Rvalue::DynamicCall`, a reference, and a deref,
    so monomorphization, borrow checking, and code generation need no protocol
    variant. A concrete call becomes `CallTarget::Direct`, a bound call remains
    symbolic until its instance is substituted, and a `dyn Index`/`dyn IndexMut`
    receiver uses the existing vtable slot. Monomorphization rejects any
    surviving `Param` or `Infer` as an internal invariant failure; type checking
    reports `E3055` first.
    A custom `for` source resolves through a peeled reference layer: a unique
    `IntoIterator` wins, and a unique direct `Iterator` is the fallback. Lowering
    evaluates the source once, calls `iter` when present, and builds the CFG
    condition -> pattern binding -> body -> `next` -> condition with `continue`
    to the condition and `break` to the exit. The item local is moved into the
    pattern; a failed refutable pattern skips the body. Custom indexing lowers
    reads to `Index` plus a deref and writes, compound assignments, and exclusive
    borrows to `IndexMut` plus a deref. The returned reference participates in
    the ordinary call-return loan rules, including the conflict checks for a
    live base borrow. Built-in arrays, slices, and ranges keep their dedicated
    bounds-checked paths and never require a protocol implementation.

71. **Global data is constant-initialized.** Every `static` and every `const`
    initializer is evaluated before the program starts; there are no global
    constructors, no initialization order, and no runtime initialization code.
    A `static` has exactly one allocation for the whole program, and a `const`
    has no storage of its own: each use is replaced by its value.
72. **Constant evaluation is deterministic and bounded.** The evaluator reads
    only source, evaluated values, and the target data model; it never reads the
    clock, environment, filesystem, or hash iteration order. Steps, call depth,
    allocations, and value size are limited, and every limit is a typed
    diagnostic rather than a hang or a panic.
73. **Constant evaluation runs before borrow checking.** `ConstProgram` is
    produced from checked MIR and records values, allocations, relocations, and
    dependency edges. Borrow checking, monomorphization, and code generation
    consume those recorded facts and never re-evaluate a constant.
74. **Mutable globals are explicit and unsafe-gated.** Only `static mut` names
    writable global storage, every access to it requires an enclosing
    `unsafe { ... }` block, and a reference to it may not escape its function.
    The block gates nothing else and has no runtime representation.
75. **Global accesses are checked around calls.** A whole-program fixed-point
    analysis records which statics a function may read or write, unions the
    effects of its callees, and treats trait-bound and dynamic calls
    conservatively; the borrow checker applies those summaries at call sites.
76. **Global emission is ordered.** Reachable statics, const allocations, and
    vtables are collected deterministically before code generation; the backend
    declares every global before initializing it, so relocations always name a
    declared symbol.
77. **Every value definition owns one initializer body.** A file-scope or local
    `const` has a `BodyKind::Const` body, and a file-scope or local `static` has
    a `BodyKind::Static { mutable }` body matching its definition. The body has
    no parameters and its return type is the declared type, so consumers never
    read an initializer expression outside a body.
78. **A value declaration follows `let` scoping.** `const` and `static` enter
    the value namespace after their initializer is lowered, shadow an outer
    value in an inner rib, and are rejected as `E2068` when they collide with
    another value in the same rib or with a file-scope function.
79. **A local value captures nothing.** A local `const` or `static`
    initializer may name file-scope items only; naming a parameter, binding,
    local value, generic parameter, or `Self` from the enclosing function is
    `E3060`, because the item is evaluated outside the call frame. The check
    runs during resolution, so a captured name never becomes a `Res`.
80. **Globals are emitted as exact memory images.** A `static` and an addressed
    `const` are encoded from their evaluated value into a byte buffer with one
    symbolic pointer slot per address, and the global's LLVM type is a packed
    struct of byte runs and pointer fields carrying the language alignment.
    Every slot is resolved after all globals are declared, so a static may
    refer to a later static or to itself, and an immutable static that holds a
    pointer lands in read-only-after-relocation data. A static is never
    `unnamed_addr`, a zero-sized allocation keeps one byte so its address stays
    unique, and no constructor or thread-local storage is emitted for any
    global.
81. **Drop sites are explicit MIR statements.** Every lexical scope exit, early
    return, loop exit, and temporary owns a `Drop` statement where the value is
    released, and elaboration replaces an unreachable site with nothing and
    guards a conditional one with a `bool` flag that assignments set, moves and
    drops clear. An assignment to an initialized place whose type needs a
    destructor drops the old value first (`DropKind::Overwrite`). No phase after
    elaboration invents or removes a release.
82. **Drop glue is one function per concrete type.** Monomorphization collects
    one `void (ptr)` glue for every concrete type it must release, calls the
    recorded destructor first, and then lists the fields, the active variant, or
    the array elements that follow. Code generation emits each glue with the
    `seal.drop.glue.<fingerprint>` symbol, and the first slot of every vtable
    points at the glue of the concrete type, including a type that declares no
    destructor, because the slot has no fallback.
83. **Destructors never run at compile time nor for statics.** Constant
    evaluation rejects a drop that would run a destructor (`E4028`) and never
    executes user code as a destructor, and a `static` is initialized before the
    program starts and is never released. Only a value a runtime scope owns is
    dropped, and a panic ends the program without unwinding.
84. **Raw pointers are a type, not a reference.** `*raw T` and `*raw mut T`
    are distinct interned types with `RawPtrKind`; both are `Copy`, `Sized`,
    nullable, and never unify with references. A sized pointee is one word; a
    string, slice, or trait object is the same two-word `{ data, metadata }`
    shape as its reference counterpart.
85. **Pointer casts are classified once.** Type checking, MIR, and code
    generation share `CastKind::{RefToRaw, RawToRaw, RawToInt, IntToRaw}`.
    Reference casts preserve pointee and may only discard permission,
    raw-to-raw casts keep compatible metadata, and only pointer-sized integers
    cross the pointer boundary. The backend maps the pointer cases to identity
    or `ptrtoint`/`inttoptr`, never to a language-level reinterpretation.
86. **Unsafe is explicit.** `unsafe { ... }` gates raw dereference and
    projection, raw writes and copies, non-wrapping raw arithmetic,
    `offset_from`, mutable-static access, and calls to `unsafe func`; it does
    not bypass type, initialization, borrow, or effect checking. The unsafe
    depth is tracked lexically, and an unsafe function body still opens its own
    block.
87. **Pointer provenance is preserved.** Field, array, and raw offsets use
    GEPs; an internal offset never round-trips through an integer. `addr`,
    `expose_addr`, `with_addr`, explicit address casts, pointer equality, and
    `offset_from` are the only value observations that may use
    `ptrtoint`/`inttoptr`. Inbounds operations use inbounds GEPs; `wrapping_*`
    uses the non-inbounds form.
88. **Raw access is a first-class MIR operation.** `Projection::RawDeref`, raw
    pointer intrinsic calls, and `unsafe` call sites are visible to MIR
    validation and to the effect/alias analysis. A body that touches raw storage
    withholds `noalias` from every parameter whose exclusive grant could be
    violated.
89. **Raw constants are evaluated, not copied from source.** Constant
    evaluation records `RawPtr`, `RawSlice`, `RawStr`, `RawStrData`, and
    `RawDyn` values with allocation provenance. A pointer to a static is a
    relocation; `p as usize`, `addr`, and `expose_addr` are `E4030`; writes are
    `E4031`; invalid reads are `E4032`. A null or integer-built pointer may
    still be stored, queried, and compared.
90. **Raw pointer equality is address equality.** `==` and `!=` compare the
    data word only; length and vtable metadata do not participate. Ordering
    operators remain unavailable, and code generation does not imply a value
    comparison for a raw pointer.
91. **Copy implementations are validated once and queried safely.** HIR
    lowering validates every `impl Copy` against the declaration: parameters in
    order, no specialization, one implementation per ADT, no destructor, and
    every field of every variant. Surviving implementations are recorded in
    `TyCtxt`; `is_copy_with` combines that registry with the body's `ParamEnv`,
    walks the fields with a `(AdtId, SubstsId)` guard, and never loops on a
    recursive declaration. A `Copy` implementation creates no monomorphization
    or code-generation item.
92. **Callable values keep one representation from HIR to machine code.** A
    `func` type is an interned `FnPtrDef`; a closure is a `BodyKind::Closure`
    body plus an `AdtKind::Closure` environment. Capture storage and closure
    kind are recorded once by type checking. MIR carries
    `CallTarget::Closure`/`CallTarget::FnTrait`, monomorphization resolves them
    to concrete instances or pointers, and code generation never re-infers a
    kind or reconstructs an environment. Capture-free closures cross to `func`
    only through the internal trampoline, which preserves the internal Seal ABI;
    the C trampoline remains a separate path for `*raw extern "c" func`.

## Global data and constant evaluation

Global data spans four storage classes with different contracts:

| Form | Storage | Address | Initialization | Write |
|------|---------|---------|----------------|-------|
| `const` | none; inlined per use | not guaranteed, materialized by `&` | compile time | never |
| `static` | one allocation | stable for the program | compile time | never |
| `static mut` | one allocation | stable for the program | compile time | inside `unsafe` only |
| promoted temporary | one allocation per promotion | stable | compile time | never |

A `const` may be declared at file scope or in a block; a `static` follows the
same scoping, and a block-scope static is one allocation shared by every call
and every instantiation of its enclosing function. Const and static items are
order-independent and may refer to each other, subject to the dependency cycle
check in the evaluator.

The evaluator interprets checked MIR with an explicit value model: scalars,
arrays, tuples, structs, enums, strings, slices, references, and trait objects.
Mutable references are rejected in constant values, and a reference to
interpreter-only storage may not escape evaluation. Trait objects record the
concrete type and trait so monomorphization can collect the vtable that
code generation initializes.

Mutable static storage is checked twice. The `unsafe` block gates the syntactic
access and records programmer responsibility. Borrow checking then tracks loans
rooted in a static and rejects a reference to a `static mut` that escapes its
function (`E5012`). Around calls, the global effect summary rejects a call that
may read an exclusively borrowed static (`E5011`) or write a borrowed static
(`E5013`). The analysis unions effects through direct, symbolic, recursive,
trait-object, and trait-bound calls; disjoint field paths stay separate, while
runtime indices make the whole static affected.

## Raw pointers and unsafe

Raw pointers are another type kind, not a second memory model. The pipeline
handles them explicitly at every stage:

- **Parse and HIR.** `*raw T` and `*raw mut T` parse as a distinct type with a
  contextual `raw` token. A raw dereference is a `RawDeref` projection; a cast
  is recorded as a `CastKind`; a method call becomes an intrinsic operation.
- **Type check.** The checker resolves the pointer kind, receiver-by-value
  rules, pointee sizedness, the `unsafe` gate, and the raw cast matrix. `*raw T`
  can be read but not written; `*raw mut T` permits writes. A fat pointer must
  call `data()` before arithmetic or a thin cast.
- **Const evaluation.** The value model records allocation, offset, metadata,
  and mutability. `RawPtr`, `RawSlice`, `RawStr`, `RawStrData`, and `RawDyn`
  are encoded into relocations or string slots by code generation. Constant
  reads validate nullness, alignment, bounds, and initialization; writes and
  address observations are rejected (`E4030`–`E4032`).
- **Borrow check and effects.** A raw pointer has an origin set but no checked
  loan. The body facts record that it may touch memory, and the effect analysis
  propagates that fact through calls. A function whose body may touch raw
  storage, call an `unsafe func`, or perform a dynamic call is excluded from
  `noalias`; safe shared parameters remain `readonly`.
- **MIR and codegen.** A raw dereference is `Projection::RawDeref`; raw
  intrinsics are `Rvalue::Intrinsic` operations. The backend uses loads and
  stores, inbounds GEPs for checked arithmetic, non-inbounds GEPs for wrapping
  arithmetic, `memmove`/`memcpy` for copies, and alignment 1 for the unaligned
  forms. Only `addr`/`expose_addr`/`with_addr`, casts, equality, and
  `offset_from` observe addresses numerically.
- **LLVM boundary.** `seal-llvm-ffi` is still the only crate allowed to use
  `unsafe`; it exposes GEPs and the LLVM debug-record API. No other phase
  performs unchecked pointer construction.

## C interoperability

The C boundary is an explicit slice through every phase, with one owner for each
decision:

- **Parse and AST.** `extern` is contextual: in item position it introduces a
  block or a single function, static, C struct, or opaque type; elsewhere it is
  an ordinary identifier. The AST records the ABI string, `link_name`, `link`
  directives, variadic markers, and C-struct declarations. `*raw extern "c"
  func(…) -> R` is a distinct type with an interned signature.
- **HIR.** Lowering creates `DefKind::ExternFunction` and
  `DefKind::ExternStatic`, storing an `ExternFunction` or `ExternStatic` with
  the resolved `Abi`, linker `Symbol`, and signature or storage type. `link`
  directives become `LinkDirective` values in source order. A C struct reuses
  ordinary aggregate lowering with `AdtRepr::C`; an opaque type becomes
  `TyKind::Opaque`, has no layout, and is usable only behind `*raw`. Later
  phases never re-read the source for any of this.
- **Type check.** `Abi::from_name` accepts `"c"`, `"cdecl"`, and `"system"`
  and rejects every other spelling (`E3080`). One FFI-safety predicate covers
  parameters, results, struct fields, static types, and function-pointer
  positions. The checker validates symbols (`E3082`), compares duplicate
  declarations of one symbol (`E3083`), applies the `unsafe` gate (`E3070`),
  checks variadic arguments and promotions (`E3088`), and validates function
  address casts (`E3089`).
- **MIR.** An external call is `Rvalue::Call` with `CallTarget::Extern`, which
  keeps the `DefId` and instantiated signature; a callback call is
  `Rvalue::IndirectCall`. An external static is an ordinary place whose root
  resolves to the declaration. Variadic promotions are explicit operands, not a
  backend decision.
- **Code generation.** `CAbi::for_target` selects the aggregate family once per
  target. `CAbi::classify` returns `Direct`, `Small(parts)`, or `Memory`
  using `seal-layout` offsets; `CSignature` carries both the LLVM function type
  and what each position means, so a declaration and every call site use one
  cached answer. A coerced integer keeps its exact width (1 through 128 bits),
  so a 3-byte struct is `i24`, not a padded `i32`. Declarations use `external`
  linkage with the ABI-mandated `byval`/`sret` only. A `c"..."` literal is one
  private NUL-terminated constant shared with the `*str` intern cache.
- **Linking.** The driver collects source `LinkDirective`s, then `-l`
  libraries, then `--link-arg` values, and places the runtime archive last.
  Every value is one argv element, never shell text. Names and search paths are
  validated before any temporary file is written (`E6005`); a failed link is
  `E6003` and reports the command, libraries, and linker output.

The section invariants are:

1. One `DefId` produces exactly one external declaration, cached by definition.
2. One aggregate classification serves the declaration and every call site; the
   two sides cannot disagree.
3. An external declaration carries no attribute the C ABI does not mandate:
   never `noalias`, `readonly`, or `nounwind`.
4. An external static is emitted as a mutable global, because C may write it.
5. Link inputs reach the linker in a deterministic order, one argument each.
6. A program without `extern` declarations emits no external declaration and is
   unchanged in IR and behavior.

## Extension points

- **New phase.** Add a crate that depends on the session and diagnostics, exposes
  `pub fn run(session: &Session) -> Result<..., ErrorGuaranteed>`, and is called
  from `seal-driver`. Phases must not own the source map or interner; they borrow
  them from the session.
- **New code generation case.** Extend `TypeMapper` for a new type kind, or add
  an arm to `FunctionLowerer::emit_rvalue`/`load_operand` for a new rvalue. Keep
  `seal-layout` authoritative for size, alignment, and offsets, and finalize any
  new debug metadata before verification.
- **New cast family.** A new category, such as bool or enum conversion, adds a
  variant beside `NumericCastKind` and a new code generation path. It never
  reinterprets the numeric variants, and it must define its validity or
  saturation rule before the surface syntax is accepted. Numeric casts stay one
  classification table shared by type checking, MIR, and code generation.
- **New artifact.** Extend `Emit` and `artifact::emit_artifact`; write through
  `seal-fs::write_atomic` and remove temporary inputs on every path. Linker
  selection belongs in the backend, never in a language phase.
- **New diagnostic.** Allocate the next code in the phase's range in
  `seal-error-codes`, write the explanation, and emit through `DiagCtxt`. Add a
  fixture test using `seal-test-support`.
- **New syntax.** Add the node to `seal-ast` and a `push_*` method to
  `AstBuilder`, parse it in `seal-parser`, and give it a recovery path plus a
  span that covers its children. Precedence lives in `expr.rs`'s operator
  tables, so a new binary operator is one entry and one enum variant.
- **New operator.** Add the AST variant and its parser precedence, then add an
  arm to `seal-typeck`'s operator table with the operand family it accepts and
  tests for each accepted and rejected family. Operators never convert their
  operands; a mixed-type use is `E3002` or `E3001`. Finally add the matching
  `BinOp` or `UnOp` variant in `seal-mir` and lower it in `lower.rs`'s operator
  tables.
- **New pattern form.** Add the AST variant in `seal-ast`'s pattern arena, parse
  it in `seal-parser`, resolve it in `seal-hir`'s pattern lowering, type it in
  `seal-typeck`'s `check_pattern`, extend the cover/uncover algorithm in
  `exhaustive.rs`, and lower its tests in `seal-mir`'s `match_expr.rs`. A new
  form must also define its binding rule for `|` alternatives and whether a
  guard may observe it. Keep the pattern tree canonical: named fields are
  reordered into declaration order during lowering, so no later phase depends on
  source order.
- **New intrinsic method.** Add its entry to `seal-methods`, then handle the
  new `IntrinsicOp` in `seal-typeck`'s signature checks if it needs a special
  receiver rule, in `seal-codegen`, and in any borrow-check rule the operation
  requires. The catalog is the single source of truth: lookup, arity, return
  types, and MIR validation must not duplicate its data. Add a catalog test, a
  type-checking test for each accepted and rejected receiver, and an end-to-end
  test for observable behavior and panics.
- **New inherent method or associated function.** Parse it inside `impl` as a
  normal `FnDecl`, then register it in `seal-hir`'s per-type method map and, when
  its first parameter is `self`, record its receiver kind. Type checking
  resolves the receiver before MIR; MIR passes the receiver as the first
  argument, materializing `Rvalue::Ref` for an automatic borrow. No codegen
  case is needed because the call is an ordinary `Rvalue::Call`, but the
  per-type symbol name must stay unique. Add parser, HIR, typeck, MIR, borrowck,
  and end-to-end coverage.
- **New trait or lookup rule.** Traits are declarations plus implementations,
  with an object-safe subset usable behind `dyn`. Parse the declaration in
  `seal-parser`, register the trait and its `Self`-relative signatures in
  `seal-hir`, validate each impl as a contract, and resolve a static call
  through the same lookup that serves inherent methods. A default body is one
  more candidate in that lookup and falls back to the trait declaration only
  when the impl omits the name; monomorphization owns the final choice between
  the override and that default. A static trait call stays a direct
  `Rvalue::Call`; a call on an object receiver becomes `Rvalue::DynamicCall` with
  the slot chosen by the shared vtable order. A new rule changes the registry
  order and its ambiguity check or the object-safety check, never the direct-call
  shape. Add parser, HIR, typeck, MIR, borrowck,
  codegen, and end-to-end coverage.
- **New operator protocol.** Add the compiler-owned trait and its methods in
  `seal-hir::builtin`, record the selected call in `TypeckResults`, and lower
  it through the existing call/reference/place machinery. Reserve the trait
  name, reject implementations on compiler-owned types, and add static,
  symbolic, dynamic, borrowck, and end-to-end coverage. Do not add a parallel
  MIR call shape for operator sugar.
- **New MIR statement or rvalue.** Add the variant in `seal-mir`, teach
  `validate` about every place and operand it carries, lower it from `seal-hir`,
  and extend `successors` only if it can change control flow. `Rvalue::Call` is
  the template for a value-producing operation with one successor.
- **New control-flow construct.** Lower it to blocks and `Goto`/`SwitchInt`
  edges; `while` shows the loop-target stack and `&&`/`||` the short-circuit
  shape. Every new edge must go through `validate`'s target check.
- **New projection.** `Place` carries
  `Projection::{Deref, Field(DefId), Index { local, constant }, VariantField { variant, field }}`;
  a new projection (slice range, dereference of a smart pointer) is a variant in
  `seal-mir` plus a resolved counterpart in `seal-typeck`. `VariantField` is
  only valid after the tag test proved its variant active, so lowering emits it
  inside one match arm and validation keeps the invariant. Update
  `seal-borrowck`'s prefix comparison and `seal-layout` when the projection
  selects storage with an address; the walk that reads and writes places
  otherwise stays unchanged.
- **New loan kind.** A loan kind is a field on `seal-borrowck`'s `Loan` plus a
  rule in the centralized conflict check; the may-hold analysis is
  kind-agnostic. Two-phase borrows become a reservation state on the same facts.
- **New ownership rule.** Add the dataflow domain in `seal-borrowck` beside
  liveness, may-hold, and initialization, and report through the `E5xxx` range;
  `check` walks every body in block order, so a new pass slots in without
  changing the driver.
- **New aggregate kind.** Structs and enums use the `AdtDef`/`FieldDef` arena
  and `TyKind::Adt`; a new nominal form adds an `AdtKind`, reuses the same
  arena, and extends `seal-layout` with its discriminant. A structural form
  such as a tuple adds a kind plus a structural interner and extends the same
  layout, query, and move-path rules. Structural queries must keep their cycle
  guard, because declarations can refer to each other.
- **New type.** Add the `TyKind` variant, a constructor on `TyCtxt`, an
  unification arm, its `Copy`/`Sized`/`Drop` classification, its display form,
  and tests. A structural type also needs an interner so equality stays
  `TyId` equality (tuples are the reference implementation). Variants must stay `Copy`, `Eq`, and `Hash`, because they are both
  arena elements and interning keys.
- **New token or keyword.** Add the variant to `TokenKind`, classify it in the
  scanner (keywords live in `seal-lexer`'s keyword table), and add a maximal
  munch test; keywords are reserved words, not identifiers with a flag.
- **New C ABI family.** Add the family to `CAbi::for_target` and its
  classification arm; the `CSignature`, call-site coercion, and debug paths are
  already family-independent. A scalar-only target still compiles.
- **New FFI surface.** A declaration form enters the AST, HIR `DefKind`, and
  resolver, then reuses the existing FFI-safety predicate, symbol table, and
  `CSignature`. Unions, bitfields, and `extern enum` are the intended next
  forms; each needs a layout and classification rule, not a new boundary.
- **New target.** Extend `seal-target` with a `TargetSpec` built from the triple;
  never consult the host at compile time.
- **New definition kind.** Add the `DefKind` variant, create the `DefData` in
  `seal-hir` lowering, resolve it to a `Res` variant, and teach consumers
  (`Hir::place`, the checker) about it. Definition IDs are append-only, so
  adding a kind never renumbers an existing definition.
- **New namespace.** Give the new namespace its own scope stack (types, labels)
  instead of overloading the value ribs, because one spelling may name both a
  value and a type.
- **New emitter.** Implement `seal_diagnostics::Emitter`; configuration is passed
  in, not read from the environment.
- **Incremental compilation.** New phases use the query keys and dependency
  rules in [Incremental queries](#incremental-queries). A query value that
  contains spans is scoped to one `SourceMap` and therefore to one session.

## Incremental queries

`seal-query` is the incremental layer. It memoizes a pure computation by a
stable logical key and records every input and derived query that computation
read. The driver does not use timestamps, file modification times, or absolute
paths to decide whether a value is valid.

### Inputs and keys

An input is a named byte string with one of six kinds: source, manifest,
features, cfg, toolchain, or declared environment. The identity is
`(kind, name)`; installing the same bytes again keeps the revision, and
installing different bytes advances the global revision exactly once. The
process environment is never read implicitly: a caller that wants an
environment value adds it as an input.

Derived keys use one of the stable kinds below. The name is an entity identity
inside the compiler, not a host path.

| Kind | Meaning |
|------|---------|
| `tokens` | Lex one source file. |
| `ast` | Parse one token stream. |
| `module-items` | Resolve the items and imports visible in a module. |
| `signature` | Lower one declaration to its HIR signature. |
| `body-hir` | Lower one body to HIR. |
| `typeck-body` | Type-check one HIR body. |
| `mir-body` | Lower one checked body to MIR. |
| `mono-instance` | Collect one monomorphized instance. |
| `codegen-cgu` | Emit one code-generation unit. |

### Validation and backdating

A query records a dependency stamp for each read: the input digest or the
result fingerprint of another query. A request is a hit only when the key's
dependency list is unchanged and every recorded fingerprint still matches the
current value. A missing dependency is invalid, so deleting a cached producer
cannot leave a consumer using a stale value.

When an invalidated query runs, the caller supplies a semantic fingerprint for
the new result. If the fingerprint equals the previous one, the result is
**backdated**: the value is replaced, but dependents remain valid because their
recorded fingerprint still matches. A comment-only edit can therefore stop at
the body that normalizes trivia instead of forcing type checking, MIR, and code
generation again.

All values are `Arc`-shared and must be `Send + Sync`. The database never holds
its lock while a computation runs, and a thread-local active stack turns a
recursive query into `QueryError::Cycle` rather than a deadlock. Independent
queries can run concurrently. Statistics count hits, misses, and backdates per
query kind.

### Session integration

`Session` owns one `QueryDb`; every fresh session starts with an empty table.
Values can contain spans into that session's `SourceMap`, so they cannot move to
another session. `seal-driver` routes lexing, parsing, and HIR lowering through
the database (`frontend::lex`, `frontend::parse`, `frontend::lower_single`,
`frontend::lower_package`). Type checking is split into a declarations query and
one query per non-closure body, MIR lowering has one query per body, and
monomorphization queries specialization per instance. Code generation remains
the existing per-CGU action in `seal-build`, whose key hashes the mono profile and
unit digest; the `codegen-cgu` query key exists for byte-cache consumers.

The HIR query's fingerprint covers declarations only. Source text behind every
body is blanked before hashing, so a body edit recomputes the HIR value (new
spans and arena IDs) but backdates its result. Each body query then depends on
the HIR stamp, the declarations fingerprint, and a structural fingerprint of its
own body. That structural fingerprint normalizes span positions to file-local
offsets: a persistent session places a new file generation at a new global base,
and unchanged bodies must still hit even though their absolute positions
changed. Arena IDs stay in the fingerprint, so editing an earlier body that
shifts later IDs invalidates the bodies that actually shifted.

A body source edit therefore recomputes tokens, AST, the HIR value, and only the
edited body's typeck/MIR/mono instance. Declarations and every other body stay
cached. A signature edit changes the declarations fingerprint and invalidates
all body checks that depend on it.

### In-process sessions and watch

The driver keeps a process-local cache of sessions keyed by tracked options plus
presentation format and color. `seal-build` runs frontend actions through
`seal_driver::run_from` in the same process, so `seal watch` reuses the session,
source map, interner, and query database across edits instead of constructing a
new compiler. Each request resets diagnostic counts and replay state; cached
query values carry their diagnostics and replay them through the normal emitter
on a hit.

A changed file is registered as a new source generation with a fresh `FileId`,
because values from earlier versions still contain spans into the old text.
Those generations are retained while the values that reference them are
reachable. When a session accumulates more than 128 source generations, the
cache replaces it with a fresh session; the next request rebuilds from an empty
query database, bounding memory in a long watch.

### Persistent cache

`DiskCache` stores only byte payloads, with a format version, key digest, length
limit, and BLAKE3 checksum. Writes use a temporary file and rename; a corrupt,
truncated, wrong-key, oversized, or old-version record is a miss and a rebuild.
The key file name is derived from the query kind and logical name, never from an
absolute project path.

The build layer still uses its content-addressed action cache as the
authoritative artifact cache. The frontend action additionally stores its mono
artifact and link metadata in `seal/cache/query-payloads`, keyed by the plan
digest and every source digest. If the action record is lost but this payload
survives, the frontend action restores both files into the CAS without running
the compiler again. The source digests are part of the key, so an edit can never
serve a stale artifact.

### Remote trust boundary

`seal-remote` is a synchronous REAPI v2 client over a private Tokio runtime.
Every remote value crosses a trust boundary: response digests and sizes are
checked against the requested digest before bytes enter the local CAS, action
results are checked against the outputs the local action declared, and action
request paths are normalized relative paths. The wire action key is a canonical
REAPI `Action` whose salt is the local action key; its `Command` commits the
kind, inputs, hashed environment, display command, and outputs. A server cannot
alias two locally distinct actions.

Remote execution uploads a canonical `Directory` tree plus `Command`/`Action`
and consumes the `Execute` stream. Only the frontend and `codegen-cgu` actions
are remote: link and generator actions depend on host toolchains and native
libraries that are not part of their declared inputs. The client checks every
returned output before building an `ActionRecord`; a missing output, digest
mismatch, or nonzero remote exit is a typed failure. Infrastructure failures
disable remote access for the rest of the build and fall back locally, while
`--require-remote` makes the same failures fatal.

The reference `seal-remote-server` is a process-local service with persistent
content-addressed storage. It validates bearer metadata in constant time,
enforces instance names, request and blob limits, and action timeouts, and runs
actions in a private work directory through the same sandbox as local
generators. Input names, output names, working directories, and relative program
paths are validated before any join, and absolute programs are limited to the
configured tool directory, so a malicious `Directory` or command cannot escape
the work tree.

### Measurements

Measured on 2026-11-21 with `cargo bench -p seal-query` in release mode on the
Linux development host. Numbers vary with the machine and load; the counts are
deterministic.

| Scenario | Measurement |
|----------|-------------|
| Frontend, direct lex+parse | 5.105 us/iteration |
| Frontend, query-cached lex+parse | 0.698 us/iteration |
| Frontend cache | 7.31x, 1000 hits, 0 misses |
| Query graph, no-op | 5.126 us/iteration, 0 misses |
| Body edit | 7.589 us/iteration, 5 misses/edit (body, typeck, MIR, mono, CGU) |
| Signature edit | 9.411 us/iteration, 9 misses/edit |
| Comment-only body edit | 6.003 us/iteration, 1 miss/edit, backdated |
| Feature edit | 500 misses / 500 edits |
| Cfg edit | 500 misses / 500 edits |
| Const edit | 500 misses / 500 edits |
| Driver body edit (actual session) | 1 tokens, 1 AST, 1 HIR, 0 signature, 1 typeck-body, 1 MIR-body, 1 mono-instance miss; the other body hits |
| `seal watch` rebuild after a source edit | 0.01 s for the `ztests` project; codegen and link were cache hits |
| 256 KiB serializable payload | 179.052 us generate, 429.695 us disk hit (0.42x) |

The query graph is a deterministic model of body and signature invalidation,
not a full compiler measurement. Its direct-compute baseline is intentionally
trivial (1.033 us), so query overhead dominates: the benchmark proves the
invalidation cardinality, not a speedup. The disk cache is slower for a
synthetic 256 KiB payload that is cheap to regenerate; it must be applied only
where the avoided computation costs more than the read and checksum.

The driver body-edit row is measured by
`pipeline::tests::editing_one_body_reuses_the_other_bodies`, which performs two
real `check` requests through the shared session and compares query counters. It
proves invalidation cardinality end to end; it is not a wall-clock speedup. The
watch latency is reported by its existing status line. The disk cache remains a
net loss for cheap synthetic payloads, so the build uses it only after an action
cache miss and never as a replacement for the action cache.

## Verification

Every change must pass:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

The workspace forbids `unsafe_code` in every compiler crate and warns on missing
documentation. `seal-llvm-ffi` is the single audited exception: it isolates GEP
construction and the LLVM 19+ debug-record call that inkwell 0.11 exposes
unsafely or wraps incorrectly. A new public item without docs fails the build.
