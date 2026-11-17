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
| `seal-ty` | Interned semantic types, nominal aggregates, structural tuples, arrays, and slices; inference variables, unification, the `Copy`/`Sized`/`Drop` queries from the memory model, and the shared initialization lattice. |
| `seal-methods` | The closed compiler-owned intrinsic method catalog: owners, receiver modes, signatures, return kinds, and semantic operations shared by type checking, MIR, and code generation. |
| `seal-layout` | Deterministic struct, enum, tuple, and array layout: field offsets, alignment, tag and payload layout, and size derived from `TyCtxt` types. |
| `seal-def` | Definition identities: `DefId`, kind, name and spans, owner, and binding mutability. |
| `seal-hir` | AST lowering and name resolution: one body per function, inherent impl and receiver registries, trait and impl coherence, resolved paths, places, and match patterns, `E2xxx` diagnostics. |
| `seal-typeck` | Type and initialization checking: expression/local/signature types, generic argument inference and bounds, array and slice bounds, array-to-slice coercion, address-space limits, literal ranges, mutability, match guards and exhaustiveness, `E3xxx` diagnostics. |
| `seal-mir` | Control-flow-graph lowering of checked HIR: locals, blocks, statements, field, tuple, index, and variant projections, bounds checks, match dispatch, terminators, and structural validation. |
| `seal-borrowck` | Liveness, may-hold loan, and initialization dataflow over MIR: shared aliasing, moves, tuple and array element paths, escaping references, `E5xxx` diagnostics. |
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
    is never shared or orphaned.
25. **Loop control is checked.** `break` and `continue` may appear only inside a
    loop. Lowering tracks loop depth, reports `E2003`/`E2004` outside one, and
    keeps the statement so later phases still see the shape that was written.
26. **Scopes follow the source.** Resolution walks ribs from the innermost
    outwards; a `let` inserts after its initializer, so a binding is not visible
    in its own initializer, and `if`/`while`/`for` bodies never leak bindings;
    a `for` pattern is scoped to its body, an `if let` pattern is scoped to its
    then block and never its else branch, and a `while let` pattern is scoped to
    the loop body.
27. **Types are assigned without coercion.** Checked types are exactly equal, or
    one side is `!`/`Error`; there is no implicit numeric conversion (D12). A
    literal adopts the expected type only when that type belongs to the same
    numeric family, and a suffix always wins.
28. **Every value expression has a type.** The results map covers every HIR
    expression except the callee of a valid call, which is not a value because
    the language has no first-class function type. Every local has a type and
    every function has a signature.
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
    the operation that consumes them, statements keep source order, and a call
    is a statement (`Rvalue::Call`) because Seal has no unwinding, so a call
    has exactly one successor.
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
    and is never `Copy`; a tuple is `Copy` exactly when every element is. A
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
52. **Code generation consumes only checked MIR.** The driver calls the backend
    only after type checking and borrow checking report no errors, and MIR
    validation succeeds first. A layout failure, an unresolved inference type,
    or invalid generated IR is an internal compiler error, never an artifact.
53. **Every local has storage.** Locals, parameters, and the return place are
    `alloca` slots; reads and writes are loads and stores, and calls use the
    checked signature. Aggregates are built with `insertvalue` in declaration
    order, struct fields and tuple elements are projected with struct GEPs, and
    arrays and slices with a bounds-checked element offset, so evaluation order
    and layout agree.
    A slice range is lowered to explicit checks followed by a fat-pointer
    construction; `len` reads the length member.
54. **The ABI is compiler-owned.** A free function uses the `seal.<name>`
    symbol and an impl function uses `seal.<Type>.<name>`, so same-named methods
    on different types cannot collide. The C `main` wrapper is the only `main`,
    and only a free function can be the language entry point. A user `main`
    takes no parameters and returns `i32` or `()`. `*str`, `*[T]`, and
    `*mut [T]` are `{ data, length }`; `()`/`!` are zero-sized structs; the
    object is position independent.
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
    unknown target, and every call site proves its reference arguments are
    known and disjoint. A function outside the set keeps the ordinary ABI and
    remains correct; the attribute never decides whether a program is accepted.
    A shared `*T` parameter is always `readonly`, because the type system
    forbids writing through it.
63. **Methods resolve statically through per-type and catalog lookup.** A method
    call is a dedicated AST and HIR node, not a desugared field access. Type
    checking first looks up `(struct, name)` in the inherent impl registry and
    then, for non-struct receivers, the intrinsic catalog after following the
    receiver's reference layers. It validates the signature and writability and
    records whether the call is a user function or an intrinsic, the receiver
    adjustment, dereference count, and result type. A user method lowers to a
    direct `Rvalue::Call` whose first argument is the receiver, with an explicit
    `Rvalue::Ref` when the receiver is borrowed automatically; an intrinsic
    lowers to `Rvalue::Intrinsic`. Code generation consumes the recorded form
    and never re-infers ownership or the receiver shape. The two lookups are
    ordered, so a struct method shadows an intrinsic with the same name. Trait
    methods add one layer to the nominal lookup: the inherent registry wins,
    then the functions bound by the type's `impl Trait` blocks, and a name two
    implemented traits declare is ambiguous (`E2028`). Each implementation is
    validated as a contract (`E2021`–`E2026`) with `Self` substituted by the
    implementing type, and it lives beside the inherent registry rather than in
    it, so an inherent method and a trait method may share a name. A `*mut T`
    receiver passed to a shared method is reborrowed as `*T`, lowering to a
    shared `Rvalue::Ref` of the pointee.
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
  not a new type kind. Parse the declaration in `seal-parser`, register the trait
  and its `Self`-relative signatures in `seal-hir`, validate each impl as a
  contract, and resolve calls through the same static lookup that serves
  inherent methods. A trait call must stay a direct `Rvalue::Call`; a new rule
  changes the registry order and its ambiguity check, never the MIR shape. Add
  parser, HIR, typeck, MIR, borrowck, and end-to-end coverage.
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
