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
4. **Room to grow.** Adding arrays, slices, generics, methods, visibility, and an
   `unsafe` layer must not require redesigning the type system.

## Model

Seal is a **value-semantics language with moves**. Every value has exactly one
owner. Assignment, argument passing, and return move the value unless its type is
`Copy`, in which case it is duplicated bitwise. A `struct` is a **nominal**
aggregate that owns its fields; the aggregate itself is never `Copy` in v0.1,
which makes every whole-value transfer a visible move. A `Copy` field is still
copied when it is read by name. `*T` is a **shared, non-owning,
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
`*T`/`*mut T`. Not copied: `str` (unsized) and every struct value, even one whose
fields are all `Copy`. Keeping aggregates non-`Copy` means ownership is explicit
at every aggregate boundary; field reads still copy `Copy` fields individually.
`Copy` and `Drop` are mutually exclusive, and v0.1 has no user-defined copy
hooks. A future opt-in `Copy` for structs derives the property from the fields
without changing the move path model.

### D3 — Ownership and destruction

Each value has one owner. Locals are destroyed in reverse declaration order at
scope exit, including exits through `return` and `break`; aggregate fields are
destroyed in declaration order. A move transfers the destruction obligation. No
exit path may skip a live owner's destructor, and no value is destroyed twice.
Safe code cannot leak on purpose (`mem::forget` does not exist).

### D4 — `*T` is a reference, not a raw pointer

A `*T` value is non-null, has no arithmetic, cannot be cast to or from an
integer, and in v0.1 is read-only. It is created with `&place` and read with
`*reference`; it denotes a borrowed place that remains valid and initialized for
the whole region in which it is used. A dereference of a reference value denotes
a place too, so `&*reference` borrows the pointee and the reference is held in a
temporary. Exclusive access uses `*mut T`. Raw
pointers (nullable, arithmetic, provenance-erasing) are a separate type with a
separate syntax and require `unsafe`.

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

### D8 — Bindings are immutable by default

`let x = 1;` binds `x` immutably: assignment, compound assignment, `++`/`--`, and
any other write through the name are errors. `let mut x = 1;` opts in to
mutation. Parameters are immutable; a function that needs to mutate an argument
binds it again with `let mut`. A binding declared without an initializer (D9) is
the exception: assigning to it for the first time initializes it, and only
subsequent writes require `let mut`.

Binding mutability is independent of ownership: an immutable binding can still
own a value and move it elsewhere, and making a binding mutable does not change
how its value is borrowed. `*T` stays read-only regardless of the binding;
exclusive access requires `*mut T`.

Alternatives rejected: mutable by default (weaker intent, and switching after
users write code is a breaking change), per-place mutability flags (more syntax
for no additional guarantee).

### D9 — Initialization is tracked

`let x: T;` creates uninitialized storage of a known type. Reading it before
assignment is an error; assignment initializes it; a move leaves the place
uninitialized again. The checker performs definite-initialization analysis over
the control-flow graph, so `let x: i32; if c { x = 1 } else { x = 2 } x` is
accepted and a path that skips both assignments is not.

Initialization is tracked per **move path**, not per local: fields of a struct
initialize independently. `let mut p: Point; p.x = 1; p.y = 2;` initializes `p`
field by field, reading `p.x` before assigning it is an error even after `p.y` is
set, and a whole assignment `p = Point { … }` initializes every field at once.
Writing a field requires the binding to be `mut`, because a field write does not
initialize the binding as a whole; an uninitialized immutable binding is the
only case where a first whole assignment is allowed without `mut`. Moving one
field out marks that path moved and leaves its siblings usable.

### D10 — Values are sized

Locals, parameters, return values, by-value operands, and struct fields must have
a size known at compile time. `str` is the only unsized type in v0.1 and can only
appear behind a reference. Recursive types are only possible through a
reference, which indirection makes finite; a struct that contains itself by value
is `E3016`.

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

### D13 — Structs are nominal and their layout is deterministic

`struct Point { x: i32, y: i32 }` declares a nominal type: two structs with the
same fields are distinct and never unify. A literal `Point { x: 1, y: 2 }` must
initialize every field exactly once, and a type name is never a value (`E2010`).
Field access applies to a place; there is no autodereference, so a reference
field is read as `(*p).x` and a field of a temporary is rejected. Fields have no
separate visibility in v0.1, and adding visibility later does not change the
layout.

`seal-layout` assigns offsets in declaration order with natural alignment and no
reordering, so the layout is deterministic and reproducible. The struct owns its
fields: moving the struct moves all of them, and moving one field out leaves the
rest initialized, which is what the move-path initializer tracks (D9). A borrow
may select a field (`&p.x`), which is what lets a sibling stay writable (D6).

## Type-system encoding

`seal-ty` encodes these decisions instead of leaving them to prose:

- `TyKind` has `Bool`, `Char`, `Str`, `Int`, `Uint`, `Float`, `Ref(TyId)`,
  `Adt(AdtId)`, `Unit`, `Never`, `Error`, and `Infer`. `Ref` is D4's shared
  reference; there is no nullable reference kind, because nullability is not a
  type.
- Struct declarations live in a `TyCtxt` arena as `AdtDef`/`FieldDef` records,
  so `AdtId` identifies a nominal declaration while the definition holds the
  ordered fields, their `DefId`s, names, and types.
- `TyCtxt::is_copy`, `is_sized`, `needs_drop`, and `contains_reference` walk
  aggregate fields with a cycle guard; no query can loop on a malformed
declaration.
- `Str` is a distinct kind, not an alias for `*u8`, because it is unsized and
  carries a length (D7).
- `Copy`, `Sized`, and `Drop` are **queries** (`TyCtxt::is_copy`,
  `is_sized`, `needs_drop`), not flags stored in the type. They are derived from
  the type's structure, so they cannot disagree with it. Struct values are not
  `Copy` in v0.1 (D2), so the query returns `false` for `TyKind::Adt`.
- `seal_ty::InitPaths` is the shared definite-initialization lattice; type
  checking keys it by `DefId`, borrow checking by MIR `LocalId`, and both query
  the same nearest-entry state over field paths.
- Types are interned: two types are equal exactly when their `TyId`s are equal.
  During inference, equality is not enough, so `InferCtxt::unify` is the only
  comparison that decides assignability; `==` on IDs is for canonical types.
- `TyKind::Error` may only be created after a diagnostic was emitted; unification
  absorbs it so one syntax error does not produce a cascade.
- There are no region parameters in `TyKind`. Regions belong to the borrow
  checker and never appear in the type checker's types.

## Consequences for later phases

- **HIR** must represent *places* (base plus projections), not only values,
  because moves, assignments, borrows, and drops operate on places. HIR keeps
  field names symbolic; lowering resolves them once types are known.
- **Name resolution** gives every place a `DefId`; the borrow checker never
  resolves syntax.
- **Type checking** decides the type of every expression and place, resolves
  every field projection, runs definite-initialization analysis over move paths,
  and inserts drop obligations.
- **Borrow checking** runs on MIR, where every move, borrow, and drop is an
  explicit statement in a control-flow graph; regions are inferred from the
  liveness of those statements, and place conflicts compare path prefixes so
  field borrows are as precise as the moves they guard.
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
8. A struct literal initializes every field exactly once; unknown, missing, and
   duplicate fields are rejected (`E2006`–`E2009`).
9. A struct value is never `Copy` in v0.1; moving one field out leaves its
   siblings initialized and usable.
10. A struct cannot contain itself by value (`E3016`), and every field is
    `Sized` (`E3010`).
11. Layout is deterministic: declaration order, natural alignment, no
    reordering, so offsets are stable across phases and targets with the same
    data model.

## Reserved syntax

These are layers above v0.1, not gaps in it: each has its semantics defined
above and only waits for the surrounding features.

- `*mut T` — exclusive reference, with D6 aliasing.
- Generics, methods, visibility, arrays, and `Drop` for aggregates — layers on
  the nominal struct foundation, not redesigns of it.
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
