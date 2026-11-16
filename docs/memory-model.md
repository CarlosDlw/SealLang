# Seal memory model

Status: accepted. Applies to the language surface above the parser and to every
later phase (name resolution, type checking, borrow checking, MIR, codegen).

## Goals

1. **Memory safety without a runtime.** No garbage collector, no reference
   counting, no tracing.
2. **Predictable cost.** No hidden allocation, no hidden deep copy, no hidden
   synchronization; every cost is visible in the source.
3. **No lifetime annotations.** Users never write region parameters; regions are
   inferred from liveness.
4. **Room to grow.** Adding structs, arrays, slices, generics, and an `unsafe`
   layer must not require redesigning the type system.

## Model

Seal is a **value-semantics language with moves**. Every value has exactly one
owner. Assignment, argument passing, and return move the value unless its type is
`Copy`, in which case it is duplicated bitwise. `*T` is a **shared, non-owning,
non-null reference**; exclusive access requires `*mut T`. References are checked
statically by a borrow checker that computes regions from liveness, so no
lifetime syntax exists. Values live on the stack, in static read-only memory, or
in explicitly allocated heap memory; allocation is never implicit. Owning values
are destroyed deterministically when their owner goes out of scope.

## Decisions

### D1 — Moves are the default

`let b = a;` moves `a` unless `a`'s type is `Copy`. A moved-from place cannot be
read, assigned, or dropped again; reading it is an error.

Alternatives rejected: implicit deep copy (hidden cost proportional to data
size), implicit reference (prevents deterministic destruction and zero-cost
abstraction), copy-on-write (unpredictable aliasing and allocation).

### D2 — `Copy` is a property of the type

Copied bitwise: all integer and float types, `bool`, `char`, `()`, `!`, and
`*T`/`*mut T`. Not copied: `str` (unsized), and every aggregate that is not
entirely `Copy` or that has a destructor. `Copy` and `Drop` are mutually
exclusive. v0.1 has no user-defined copy hooks.

### D3 — Ownership and destruction

Each value has one owner. Locals are destroyed in reverse declaration order at
scope exit, including exits through `return` and `break`; aggregate fields are
destroyed in declaration order. A move transfers the destruction obligation. No
exit path may skip a live owner's destructor, and no value is destroyed twice.
Safe code cannot leak on purpose (`mem::forget` does not exist).

### D4 — `*T` is a reference, not a raw pointer

A `*T` value is non-null, has no arithmetic, cannot be cast to or from an
integer, and in v0.1 is read-only. It denotes a borrowed place that remains
valid and initialized for the whole region in which it is used. Exclusive access
uses `*mut T`. Raw pointers (nullable, arithmetic, provenance-erasing) are a
separate type with a separate syntax and require `unsafe`.

This keeps the published `*T` syntax and makes the default safe: the common case
does not need `unsafe`, and the unsafe layer can be added without changing
existing programs.

### D5 — Regions are inferred

There are no lifetime parameters in the source. The borrow checker assigns each
reference a region: the set of program points where the reference may still be
used (its liveness), closed under outlives constraints. A returned reference must
be backed by a region that outlives the call. Violations are reported before MIR
optimization, so no invalid access can be optimized away.

Alternatives rejected: lexical scopes (rejects valid programs), no static
checking (makes the default unsafe), a GC (violates goal 1).

### D6 — Aliasing: shared XOR exclusive

At any point a place is either shared-borrowed any number of times or
exclusively borrowed once. Creating an exclusive borrow invalidates live shared
ones for its region, and vice versa. Two-phase borrows (reserve, then activate)
are reserved for nested method-style calls once methods exist.

For the future `unsafe` layer, the runtime aliasing model is **Tree Borrows**
with pointer provenance (RFC 3559): every pointer carries the allocation and the
permission it was derived from, and accesses outside that permission are
undefined behavior. Tree Borrows accepts more real-world patterns than Stacked
Borrows and is the model Miri uses for new work.

### D7 — Allocation is explicit; no GC

There is no implicit heap. String literals are `*str` fat pointers (data and
length) into static read-only memory, so `str` is an **unsized pointee** and is
only usable behind a reference. Owned heap data arrives with the allocator API
as a distinct owning type that implements `Drop`; allocation failure is returned
as a value, not hidden behind an abort. Reallocation and growth are explicit
operations of that type, never implicit on assignment.

### D8 — Bindings are mutable by default

`let a1 = 0; a1 += 10;` is valid, as the language examples require. Immutability
is expressed by types and references (`*T` is read-only), not by the binding.
The cost is that accidental mutation is not caught by the binding; a lint can
flag bindings that are never reassigned.

### D9 — Initialization is tracked

`let x: T;` creates uninitialized storage of a known type. Reading it before
assignment is an error; assignment initializes it; a move leaves the place
uninitialized again. The checker performs definite-initialization analysis over
the control-flow graph, so `let x: i32; if c { x = 1 } else { x = 2 } x` is
accepted and a path that skips both assignments is not.

### D10 — Values are sized

Locals, parameters, return values, and by-value operands must have a size known
at compile time. `str` is the only unsized type in v0.1 and can only appear
behind a reference. Recursive types are only possible through a reference, which
indirection makes finite.

### D11 — No address-dependent semantics in safe code

Equality compares values, not addresses. Padding bytes are not observable. There
is no guarantee that a local is zero-initialized. Two equal values are
interchangeable in safe code, which is what allows moves and copies to be
reordered.

### D12 — No implicit numeric conversions

Integer literals take the type their context requires; otherwise two numeric
types only unify when they are identical. Mixed-width arithmetic is an error.
Widening and narrowing require explicit casts (added with the cast syntax), so
no silent truncation or sign change can happen.

## Type-system encoding

`seal-ty` encodes these decisions instead of leaving them to prose:

- `TyKind` has `Bool`, `Char`, `Str`, `Int`, `Uint`, `Float`, `Ref(TyId)`,
  `Unit`, `Never`, `Error`, and `Infer`. `Ref` is D4's shared reference; there is
  no nullable reference kind, because nullability is not a type.
- `Str` is a distinct kind, not an alias for `*u8`, because it is unsized and
  carries a length (D7).
- `Copy`, `Sized`, and `Drop` are **queries** (`TyCtxt::is_copy`,
  `is_sized`, `needs_drop`), not flags stored in the type. They are derived from
  the type's structure, so they cannot disagree with it.
- Types are interned: two types are equal exactly when their `TyId`s are equal.
  During inference, equality is not enough, so `InferCtxt::unify` is the only
  comparison that decides assignability; `==` on IDs is for canonical types.
- `TyKind::Error` may only be created after a diagnostic was emitted; unification
  absorbs it so one syntax error does not produce a cascade.
- There are no region parameters in `TyKind`. Regions belong to the borrow
  checker and never appear in the type checker's types.

## Consequences for later phases

- **HIR** must represent *places* (base plus projections), not only values,
  because moves, assignments, borrows, and drops operate on places.
- **Name resolution** gives every place a `DefId`; the borrow checker never
  resolves syntax.
- **Type checking** decides the type of every expression and place, runs
  definite-initialization analysis, and inserts drop obligations.
- **Borrow checking** runs on MIR, where every move, borrow, and drop is an
  explicit statement in a control-flow graph; regions are inferred from the
  liveness of those statements.
- **Codegen** may assume that no safe reference is null or dangling, that a
  `Copy` value has no destructor, and that exclusive accesses do not overlap.

## Invariants

1. No implicit deep copy and no implicit allocation.
2. Every live `*T` is non-null and points to initialized, still-live memory.
3. A moved-from place is never read, reassigned as a read, or dropped.
4. Every initialized owner is dropped exactly once on every exit path.
5. `Copy` and `Drop` never hold for the same type.
6. A `Sized` value never contains an unsized value directly.
7. Type layout and addresses never affect the meaning of safe programs.

## Reserved syntax

These are layers above v0.1, not gaps in it: each has its semantics defined
above and only waits for the surrounding features.

- `*mut T` — exclusive reference, with D6 aliasing.
- Raw pointer type — nullable, arithmetic, provenance rules from RFC 3559, only
  usable inside `unsafe`.
- Owned heap buffer and allocator API — D7's owning type.
- Explicit casts — D12's widening and narrowing.

## References

- Rust Compiler Development Guide, *The ty module: representing types* and
  *Memory management in rustc* (interning, arenas, canonical types).
- Rust Compiler Development Guide, *The borrow checker* and *Region inference
  (NLL)* (liveness-based regions, outlives constraints).
- RFC 3559, *Rust has provenance*.
- Villani et al., *Tree Borrows* (aliasing model for unsafe code).
- The Rust Reference, drop order and move semantics.
