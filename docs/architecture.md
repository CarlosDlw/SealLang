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
| `seal-ty` | Interned semantic types and aggregates, inference variables, unification, the `Copy`/`Sized`/`Drop` queries from the memory model, and the shared initialization lattice. |
| `seal-layout` | Deterministic struct layout: field offsets, alignment, and size derived from `TyCtxt` types. |
| `seal-def` | Definition identities: `DefId`, kind, name and spans, owner, and binding mutability. |
| `seal-hir` | AST lowering and name resolution: one body per function, resolved paths and places, `E2xxx` diagnostics. |
| `seal-typeck` | Type and initialization checking: expression/local/signature types, literal ranges, mutability, `E3xxx` diagnostics. |
| `seal-mir` | Control-flow-graph lowering of checked HIR: locals, blocks, statements, terminators, and structural validation. |
| `seal-borrowck` | Liveness, may-hold loan, and initialization dataflow over MIR: shared aliasing, moves, escaping references, `E5xxx` diagnostics. |
| `seal-codegen` | LLVM lowering of checked MIR, DWARF emission, optimization, and object/assembly/executable artifacts with the embedded runtime (`E6xxx`). |
| `seal-llvm-debug` | Isolated FFI bridge for the LLVM 19+ debug-record API that inkwell 0.11 mis-wraps; the only crate that permits `unsafe`. |
| `seal-version` | `LanguageVersion` and feature gates. |
| `seal-target` | Target triples and the properties derived from them. |
| `seal-fs` | UTF-8 source loading and atomic artifact writes. |
| `seal-profiling` | Phase timings for `--time-passes`. |
| `seal-session` | Owns options, target, source map, interner, diagnostics, gates, cancellation, profiler. |
| `seal-test-support` | Inline `//~` fixture annotations and structured matching. |
| `seal-driver` | The `sealc` binary: clap-based argument groups, exit codes, panic policy, and phase wiring. |

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
    passes an occurs check, and no implicit conversion exists: distinct scalar
    types never unify. `Error` is the only type that absorbs another, and it is
    only built after a diagnostic was reported.
19. **Memory classification is derived.** `Copy`, `Sized`, and `Drop` are
    queries over the interned type, never flags stored beside it, so they cannot
    contradict the type or the memory model.
20. **Bindings are immutable unless declared `mut`.** The lexer reserves `mut`,
    the AST records the flag per binding, and no later phase infers mutability
    from how a name is used.
21. **Parsing terminates and recovers.** Every loop asserts progress, recursion
    is bounded by `MAX_DEPTH`, and recovery synchronizes at `;`, `}`, `func`, and
    end of file. The same token stream always produces the same tree, and a
    malformed construct costs one error rather than a cascade.
22. **Control flow is structured.** `if` is an expression with block branches,
    `else if` is a block whose tail is another `if`, `while` is a statement with
    a block body, and `break`/`continue` target the innermost loop. A block-like
    expression used as a statement needs no semicolon.
23. **Every HIR name is resolved.** A `Path` holds a `Res` (a `DefId`), never a
    `Symbol`; a name that does not resolve becomes an `Error` node after `E2002`
    was reported, so no consumer performs a textual lookup.
24. **HIR is total and owner-partitioned.** Every function owns exactly one
    body, every ID resolves, every node is reachable from its body, and every
    parameter or binding definition records the function that owns it. A body
    is never shared or orphaned.
25. **Loop control is checked.** `break` and `continue` may appear only inside a
    loop. Lowering tracks loop depth, reports `E2003`/`E2004` outside one, and
    keeps the statement so later phases still see the shape that was written.
26. **Scopes follow the source.** Resolution walks ribs from the innermost
    outwards; a `let` inserts after its initializer, so a binding is not visible
    in its own initializer, and `if`/`while` bodies never leak bindings.
27. **Types are assigned without coercion.** Checked types are exactly equal, or
    one side is `!`/`Error`; there is no implicit numeric conversion (D12). A
    literal adopts the expected type only when that type belongs to the same
    numeric family, and a suffix always wins.
28. **Every value expression has a type.** The results map covers every HIR
    expression except the callee of a valid call, which is not a value because
    the language has no first-class function type. Every local has a type and
    every function has a signature.
29. **Conditions are boolean.** `if` and `while` conditions are checked against
    `bool`; there is no truthiness.
30. **Initialization is definite.** A local read requires initialization on
    every path that reaches it. Assignments in both `if` branches count only at
    the join; a branch that diverges does not participate; a `while` restores
    the state from before the loop. An uninitialized immutable binding may be
    initialized once by assignment; reassigning it needs `let mut`.
31. **Stored values are sized.** Locals, parameters, and return values cannot
    have `str`; literals must fit the integer or float type selected for them,
    using the target pointer width for `isize` and `usize`.
32. **MIR is a control-flow graph.** Every body has one entry block and a return
    place at local zero; parameters are locals `1..=arg_count`, bindings and
    temporaries follow, and every local declares its type. Every block ends in
    exactly one terminator, so there is no implicit fallthrough, and validation
    rejects a body whose place or edge names a missing local or block.
33. **Evaluation order is preserved.** Operands are lowered left to right before
    the operation that consumes them, statements keep source order, and a call
    is a statement (`Rvalue::Call`) because Seal has no unwinding, so a call
    has exactly one successor.
34. **Short-circuiting is control flow.** `&&` and `||` lower to `SwitchInt`
    graphs whose short-circuit block stores the constant result, so the right
    operand is evaluated only on the path that needs it. MIR has no `BinOp` for
    them.
35. **Loops and branches are explicit.** `if`, `while`, `break`, and `continue`
    lower to blocks and edges; boolean conditions use `SwitchInt` with `1` for
    true, `continue` jumps to the condition block, and `break` to the loop exit.
    A condition is always evaluated in its own block, once per test.
36. **Lowering assumes a clean check.** MIR is produced only from a HIR tree
    that type checking accepted without errors, so every value expression has a
    type, every assignment target is a local place, and every local has a
    storage type. The driver validates the lowered tree and reports a structural
    failure as an internal compiler error.
37. **Divergence does not truncate the graph.** A `return` terminates its block,
    but lowering keeps processing the rest of the source, so unreachable blocks
    may exist and must stay structurally valid. Validation does not require
    reachability; removing dead blocks belongs to a later pass.
38. **References are checked, never annotated.** `&place` creates a `*T` loan
    and `*reference` reads through it. A dereference of a reference value is a
    place as well, so `&*identity(p)` borrows the pointee and lowering keeps the
    reference itself in a temporary. Regions are inferred from liveness; there
    is no lifetime syntax, and no region appears in a type.
39. **A shared loan permits reads and forbids writes.** `*T` is read-only, so a
    live loan conflicts only with assigning or moving the borrowed place. Two
    shared loans may overlap, and reads never conflict.
40. **Loans are tracked per point, not per region union.** A local's may-hold
    set is a forward dataflow fact: overwriting a reference drops the loans it
    carried, and a loan is active only while a live local may hold it. This
    accepts programs where a place becomes writable again after the last alias
    dies, including inside loops.
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
44. **Aggregates are nominal and structural.** A struct declaration interns an
    `AdtId`; its type is `TyKind::Adt`, never a structural tuple. Two structs
    with identical fields are different types, and `Copy`, `Sized`, `Drop`, and
    `contains_reference` are derived by walking the field types, with a cycle
    guard so a malformed declaration cannot hang a query.
45. **Struct literals are complete.** Every field appears exactly once, every
    named field exists, every field initializer has the declared field type, and
    a literal in value position is rejected in favor of a type name
    (`E2005`–`E2010`). Duplicate field names and duplicate struct names are
    rejected at declaration.
46. **Fields are places, not accessors.** `p.x` denotes storage: it needs a
    place base (`E2012`), never autodereferences a `*T` (`E3015` tells the user
    to write `(*p).x`), and selects a field whose resolved `DefId` is recorded
    once by the checker for MIR. Reading a field reads only that field's move
    path; it does not require sibling fields to be initialized.
47. **Structs own their fields and are not `Copy`.** A struct value moves as a
    whole; a `Copy` field still copies when read. Field writes require the root
    binding to be `mut` (`E3004`), because a field write does not initialize the
    binding as a whole. Field-by-field initialization needs a declared type and
    `mut`, and a whole assignment initializes every field at once.
48. **Recursion needs indirection.** A struct cannot contain itself by value
    (`E3016`); a reference field breaks the cycle. A struct field must be
    `Sized` (`E3010`).
49. **Layout is deterministic.** `seal-layout` assigns offsets in declaration
    order with natural alignment and no reordering, so the same types produce
    the same layout in every phase and on every target with the same data model.
50. **MIR places carry projections.** A `Place` is a local plus
    `Projection::{Deref, Field(DefId)}`; a struct literal lowers to
    `Rvalue::Aggregate` with field operands in evaluation order, and a non-`Copy`
    field read is a `Move`. Validation checks every projected place's local. A
    dereference of a reference value (`*identity(&v)`, `*&x`) has no local to
    project from, so lowering stores the reference in a temporary and projects
    through it; the read still copies, because the place is reached through a
    reference.
51. **Move paths reach into aggregates.** Type checking and borrow checking
    share `seal_ty::InitPaths`: a field assignment initializes only that path,
    reading an uninitialized field is `E3011`/`E5003`, and a read of a field
    whose sibling moved is allowed. Borrow conflicts compare path prefixes, so
    borrowing `p.x` leaves `p.y` writable and borrowing `p` blocks both. A
    whole-place borrow with a moved field is `E5006`.
52. **Code generation consumes only checked MIR.** The driver calls the backend
    only after type checking and borrow checking report no errors, and MIR
    validation succeeds first. A layout failure, an unresolved inference type,
    or invalid generated IR is an internal compiler error, never an artifact.
53. **Every local has storage.** Locals, parameters, and the return place are
    `alloca` slots; reads and writes are loads and stores, and calls use the
    checked signature. Aggregates are built with `insertvalue` in declaration
    order and projected with struct GEPs, so evaluation order and layout agree.
54. **The ABI is compiler-owned.** Seal functions use the `seal.<name>` symbol;
    the C `main` wrapper is the only `main`. A user `main` takes no parameters
    and returns `i32` or `()`. `*str` is `{ data, length }`; `()`/`!` are
    zero-sized structs; the object is position independent.
55. **Debug information describes the generated IR.** `-g` emits a Rust compile
    unit, subprograms, parameters, bindings, aggregate types with layout
    offsets, and a source location per statement and terminator, all finalized
    before verification and optimization.
56. **Arithmetic cannot silently invoke LLVM undefined behavior.** Integer
    division and remainder panic through the runtime on a zero divisor and on
    the signed minimum divided by `-1`, reporting the failing expression's
    source location and exiting with status 101; shift amounts are masked to the
    operand width.
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
59. **The runtime is minimal and self-contained.** The embedded C11 runtime
    allocates no memory, takes no locks, and uses no stdio, locale, or `atexit`
    handlers.
    Writes transfer exactly the requested bytes, retrying `EINTR` and partial
    writes; panics never unwind and always terminate with status 101. The
    archive is host-specific and linked only into executables, so object and
    assembly output stay target-independent. `--emit-runtime` exports the
    embedded archive, and `--runtime-archive` supplies a target archive.
60. **The command line is declarative.** `seal-driver` parses arguments with
    `clap`, one module per option group (output, optimization, target,
    diagnostics, runtime), and converts them into session options in one place.
    Rules that span arguments, such as a required input or a single `-o`, are
    reported as ordinary usage errors with status 2.

## Extension points

- **New phase.** Add a crate that depends on the session and diagnostics, exposes
  `pub fn run(session: &Session) -> Result<..., ErrorGuaranteed>`, and is called
  from `seal-driver`. Phases must not own the source map or interner; they borrow
  them from the session.
- **New code generation case.** Extend `TypeMapper` for a new type kind, or add
  an arm to `FunctionLowerer::emit_rvalue`/`load_operand` for a new rvalue. Keep
  `seal-layout` authoritative for size, alignment, and offsets, and finalize any
  new debug metadata before verification.
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
- **New MIR statement or rvalue.** Add the variant in `seal-mir`, teach
  `validate` about every place and operand it carries, lower it from `seal-hir`,
  and extend `successors` only if it can change control flow. `Rvalue::Call` is
  the template for a value-producing operation with one successor.
- **New control-flow construct.** Lower it to blocks and `Goto`/`SwitchInt`
  edges; `while` shows the loop-target stack and `&&`/`||` the short-circuit
  shape. Every new edge must go through `validate`'s target check.
- **New projection.** `Place` carries `Projection::{Deref, Field(DefId)}`; a
  new projection (array index, slice range) is a variant in `seal-mir` plus a
  resolved counterpart in `seal-typeck`. Update `seal-borrowck`'s prefix
  comparison and `seal-layout` when the projection selects storage with an
  address; the walk that reads and writes places otherwise stays unchanged.
- **New loan kind.** `*mut T` adds an exclusive kind to `seal-borrowck`'s loan
  table: extend the conflict check so exclusive loans reject overlapping reads
  and borrows, and keep the may-hold analysis unchanged. Two-phase borrows then
  become a reservation state on the same facts.
- **New ownership rule.** Add the dataflow domain in `seal-borrowck` beside
  liveness, may-hold, and initialization, and report through the `E5xxx` range;
  `check` walks every body in block order, so a new pass slots in without
  changing the driver.
- **New aggregate kind.** Structs use the `AdtDef`/`FieldDef` arena and
  `TyKind::Adt`; an enum or tuple adds an `AdtKind`, reuses the same arena, and
  extends `seal-layout` with its discriminant. Structural queries must keep
  their cycle guard, because declarations can refer to each other.
- **New type.** Add the `TyKind` variant, a constructor on `TyCtxt`, an
  unification arm, its `Copy`/`Sized`/`Drop` classification, its display form,
  and tests. Variants must stay `Copy`, `Eq`, and `Hash`, because they are both
  arena elements and interning keys.
- **New token or keyword.** Add the variant to `TokenKind`, classify it in the
  scanner (keywords live in `seal-lexer`'s keyword table), and add a maximal
  munch test; keywords are reserved words, not identifiers with a flag.
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
- **Incremental compilation.** The foundation already provides stable hashes,
  content hashes on `SourceFile`, and serializable IDs. A query engine can be
  added as a later crate without changing spans, symbols, or diagnostics.

## Verification

Every change must pass:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

The workspace forbids `unsafe_code` in every compiler crate and warns on missing
documentation. `seal-llvm-debug` is the single audited exception: it isolates the
LLVM 19+ debug-record call that inkwell 0.11 wraps incorrectly. A new public item
without docs fails the build.
