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
4. **Room to grow.** Adding traits, generics, visibility, and an `unsafe` layer
   must not require redesigning the type system.

## Model

Seal is a **value-semantics language with moves**. Every value has exactly one
owner. Assignment, argument passing, and return move the value unless its type is
`Copy`, in which case it is duplicated bitwise. A `struct` is a **nominal**
aggregate that owns its fields; the aggregate itself is never `Copy` in v0.1,
which makes every whole-value transfer a visible move. A tuple is a structural
aggregate that owns its elements and is `Copy` exactly when every element is, so
copying a tuple never hides a move of a non-`Copy` element. A `Copy` field or
element is still copied when it is read. `*T` is a **shared, non-owning,
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

Copied bitwise: all integer and float types, `bool`, `char`, `()`, `!`,
`*T`/`*mut T`, `*dyn Trait`/`*mut dyn Trait`, an array whose element type is
`Copy`, and a tuple whose elements are all `Copy`. Not copied: `str`, slices,
and `dyn Trait` (unsized) and every struct or enum value, even one whose fields
are all `Copy`. Arrays and tuples gain nothing from
an explicit opt-in because their elements already decide whether a bitwise copy
is safe. Keeping structs non-`Copy` means ownership is
explicit at every aggregate boundary; field reads still copy `Copy` fields
individually. `Copy` and `Drop` are mutually exclusive, and v0.1 has no
user-defined copy hooks. A future opt-in `Copy` for structs derives the property
from the fields without changing the move path model.

### D3 — Ownership and destruction

Each value has one owner. Locals are destroyed in reverse declaration order at
scope exit, including exits through `return` and `break`; aggregate fields are
destroyed in declaration order. A move transfers the destruction obligation. No
exit path may skip a live owner's destructor, and no value is destroyed twice.
Safe code cannot leak on purpose (`mem::forget` does not exist).

### D4 — `*T` is a reference, not a raw pointer

A reference is non-null, has no arithmetic, cannot be cast to or from an integer,
and is created with `&place` or `&mut place` and read with `*reference`; it
denotes a borrowed place that remains valid and initialized for the whole region
in which it is used. `*T` is shared and read-only; `*mut T` is exclusive and
read-write, and the two never unify. A dereference of a reference value denotes a
place too, so `&*reference` borrows the pointee and the reference is held in a
temporary. Reading a reference value copies the reference, and reading
`*reference` by value copies a `Copy` pointee; reading any other pointee is
`E3018`, because moving it would leave the owner's storage without a value. Raw
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
ones for its region, and vice versa: an exclusive loan conflicts with every
other loan of the same place and with reads, writes, and moves of it
(`E5007`/`E5008`), while a shared loan conflicts only with writes and moves
(`E5001`/`E5002`). Loans are sliced by place prefix, so `&mut p.x` leaves `p.y`
usable. The kind is part of the type (`*T` versus `*mut T`), so the two never
unify, and `&mut place` requires a writable place: a `mut` binding or a `*mut T`
dereference (`E3017`).

Conflicts compare storage, not syntax: a parallel may-alias analysis records the
storage each reference value may name, so `*p` and `*q` conflict when `p` and
`q` are copies or reborrows of the same pointer. Reborrowing is allowed because
the new loan records the loans that authorized it: using the reborrow does not
conflict with them, while using the parent conflicts with the reborrow. Two
cases stay beyond the analysis: a reference read out of memory has an unknown
target and conservatively conflicts with any live loan of storage it might name,
and distinct parameters are assumed not to alias, which is part of the
exclusive-reference contract. Diagnostics label the live borrow and its last
active use, so the reordering that fixes a conflict is visible. Receiver
borrows follow source order: an automatic receiver borrow is created before the
arguments are evaluated, and there is no two-phase reserve/activate step. A
receiver borrow that conflicts with an argument is therefore rejected rather
than postponed, for both intrinsic and user methods.

A call is an access through every reference argument: an exclusive argument
(`*mut T`, or an aggregate carrying one) gives the callee sole access to its
pointee for the whole call. The checker rejects an exclusive argument that
overlaps a live loan (`E5009`) and a call whose reference arguments may name the
same storage when one of them is exclusive (`E5010`). Argument storage is
compared with the may-name analysis, so copies and reborrows are ordered. This
is also what lets the backend treat an exclusive parameter as `noalias`: the
attribute is emitted only when the body never reaches storage through an unknown
pointer or a dynamic call and every call site proves the contract, so the
optimization can never turn safe code into undefined behavior. A method
reachable through a vtable has call sites the static proof never sees, so it is
excluded from `noalias` until a future proof covers indirect calls.

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
operations of that type, never implicit on assignment. A trait object also does
not allocate: its vtable is a constant built at compile time per concrete type
and trait reference, and coercing to it copies only the data and vtable words.

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

Initialization is tracked per **move path**, not per local: fields of a struct,
elements of a tuple, and constant elements of an array initialize independently.
`let mut p: Point; p.x = 1; p.y = 2;` initializes `p` field by field, reading
`p.x` before assigning it is an error even after `p.y` is set, and a whole
assignment `p = Point { … }` initializes every field at once. The same rule
applies to `pair.0`, `pair.1`, `a[0]`, and `a[1]`; a
write through a runtime index is more conservative and requires the whole array
to be initialized, because the compiler cannot prove which element is written.
Writing a field or element requires the binding to be `mut`, because it does not
initialize the binding as a whole; an uninitialized immutable binding is the
only case where a first whole assignment is allowed without `mut`. Moving one
field or element out marks that path moved and leaves its siblings usable.

### D10 — Values are sized

Locals, parameters, return values, by-value operands, struct fields, enum
variant fields, tuple elements, and array elements must have a size known at
compile time. `str`, slices, and `dyn Trait` are unsized and can only appear
behind a reference. A tuple is sized when every element is, an array when its
element is,
and its size must fit the target address space (`E3023`). Recursive types are only possible through a
reference, which indirection makes finite; a struct or enum that contains
itself by value is `E3016`.

### D11 — No address-dependent semantics in safe code

Equality compares values, not addresses. Padding bytes are not observable. There
is no guarantee that a local is zero-initialized. Two equal values are
interchangeable in safe code, which is what allows moves and copies to be
reordered.

### D12 — No implicit numeric conversions

Integer literals take the type their context requires; otherwise two numeric
types only unify when they are identical. Mixed-width arithmetic is an error.
Widening and narrowing exist only through the explicit `as` cast, so no silent
truncation or sign change can happen. A cast is a pure value operation: it
reads its operand once, allocates nothing, creates no loan, and does not change
the operand's place or initialization state. Casts between pointers,
references, aggregates, and validity-checked types are separate features and
remain absent.

### D13 — Nominal aggregates, structural tuples, and deterministic layout

`(i32, bool)` is a structural product type: its identity is its ordered element
types, so the same spelling means the same type everywhere. It has no name,
declaration, or fields, and its elements are addressed by position. A tuple is
`Copy` exactly when every element is, which keeps move visibility identical to
the element level. Its elements initialize, move, and borrow independently, and
`.0`/`.1` are places. A tuple pattern writes one subpattern per element and
follows references exactly like a struct pattern.

`struct Point { x: i32, y: i32 }` declares a nominal product type: two structs
with the same fields are distinct and never unify.
`enum Shape { Empty, Circle { radius: i32 }, Dot(i32) }` declares a nominal
**sum** type: every value is exactly one variant, identified by a tag, and two
enums with the same variants are also distinct. A variant carries no fields
(`Empty`), positional fields (`Dot(i32)`), or named fields
(`Circle { radius: i32 }`). A struct or variant literal initializes every field
exactly once: named fields by name, positional fields in declaration order. A
type name is never a value (`E2010`).
Field access applies to a place; there is no autodereference, so a reference
field is read as `(*p).x` and a field of a temporary is rejected. Fields have no
separate visibility in v0.1, and adding visibility later does not change the
layout.

`seal-layout` assigns offsets in declaration order with natural alignment and no
reordering, so the layout is deterministic and reproducible for structs and
tuples alike. An enum starts with
the smallest unsigned tag that addresses every variant, followed by a payload at
the next multiple of the widest variant field alignment; the payload is as large
as the largest variant and the whole value is padded to its alignment. The
aggregate owns its fields: moving it moves all of them, and moving one field out
leaves the rest initialized, which is what the move-path initializer tracks
(D9). A borrow may select a struct field (`&p.x`), which is what lets a sibling
stay writable (D6). The discriminant is read by `match` and never by field
access: a field of a variant is reachable only through a pattern that tested the
variant first, the one projection whose validity depends on control flow.

### D14 — Slices are borrowed views with a length

A slice `[T]` is an unsized pointee denoting a contiguous sequence of `T`
values. It is never owned or stored by value: `*[T]` and `*mut [T]` are fat
references carrying the element address and the element count. The count is
metadata, not a terminator scan, so `len` is O(1) and a slice can contain zero
values or any value including a zero byte.

A range `base[start..end]` denotes a sub-slice of an array or another slice. It
is a place of unsized type and must be borrowed; `&base[..]` creates a shared
view and `&mut base[..]` creates an exclusive one. Both bounds are `usize` and
are checked against the source length before the fat reference is built. The
loan is recorded on the underlying storage, not on the reference object: while a
slice view is live, the source is borrowed by the same shared/exclusive rules as
any other reference. Reborrowing a slice creates a child loan precisely because
the source reference's pointee is resolved through the may-name analysis.

Array-to-slice coercion (`&array` where `*[T]` is expected) is a reference
conversion, not a copy: it keeps the same data address, derives the length from
the array type, and records the same loan. A slice is neither `Copy`, `Sized`,
nor owning; its reference is `Copy` and carries the loan.

### D15 — Method dispatch borrows through the receiver

Method lookup has two sources. An inherent method declared in an `impl` block
is found on the receiver's nominal struct; intrinsic methods are a closed
compiler catalog over integers, floats, `bool`, `char`, arrays, and slices. In
both cases the owner is the receiver's type after every reference layer is
followed. There is no method value: a static user method lowers to a direct
`Rvalue::Call` with the receiver as its first argument, an intrinsic lowers to a
semantic `Rvalue::Intrinsic` whose operation the backend chooses, and a call
through a trait object lowers to `Rvalue::DynamicCall`, which loads a vtable slot
(D16). The receiver borrow is visible to the same checker in every form.

Traits use the same path. A trait declaration contributes signatures and
optional default bodies written against `Self`, and `impl Trait for Type` binds
each method to a concrete function or inherits the default. Type checking tries
the inherent registry first and then the trait implementations of the receiver's
type, recording the call as a direct `Rvalue::Call` to that function or default
body. Two implemented traits declaring one method name make the call ambiguous
(`E2028`), while an inherent method always wins. A static trait call therefore
owns and borrows exactly what the selected function or default body declares. The dynamic case, where the receiver's pointee is `dyn Trait`, is D16.

With generics, the receiver may be a declared type parameter. The generic body
cannot select an implementation, so the call stays symbolic until
monomorphization; the eventual concrete call owns and borrows exactly what the
selected impl declares. Moves and copies consult the body's parameter
environment: a `T: Copy` parameter is copied and stays usable, while an
unbounded `T` is moved. A declared parameter is conservatively treated as
containing a reference, so a generic aggregate never drops a loan it may carry.
The `noalias` proof runs once on the generic body and is independent of the
instantiation; because an exclusive reference to a declared parameter is
treated conservatively, such a parameter never receives the attribute, while a
concrete reference type that closes the proof does.

A user method declares its receiver explicitly. `self: T` moves or copies the
receiver, `self: *T` borrows it shared, and `self: *mut T` borrows it exclusive.
When the receiver expression is a `T` place and the method expects a reference,
the compiler creates the same loan a written `&receiver` or `&mut receiver`
would create; the borrow checker therefore sees no special case. A `*mut T`
receiver passed to a method that declares `self: *T` is reborrowed as `*T`, a
shared loan of the pointee, so a read-only method cannot observe a write while
its loan is live. An exclusive automatic borrow still requires a writable place,
and an unrelated receiver type is rejected before MIR. Associated functions have no receiver and are called as
`T::name(...)`, which resolves to an ordinary function item.

A scalar method receives a copy. A sequence method borrows its receiver: `len`
and `is_empty` take a shared loan, while `swap`, `reverse`, `fill`, and
`bool::toggle` take an exclusive loan. Borrow checking sees exactly those
accesses, in receiver-then-arguments order, so a modifying method conflicts with
a live shared or exclusive loan of the same storage under the ordinary rules
(`E5007`/`E5008`). A `*mut [T]` receiver is a value that already carries its
exclusive loan; a `*[T]` receiver cannot support a modifying method (`E3029`).

A temporary receiver is allowed because its storage exists only for the call.
The MIR lowering stores a statement-valued method result in a temporary, so a
method whose result is ignored still executes; no side effect is optimized away
by the front end. Sequence methods check every runtime index against the length
before access, and no method allocates or changes a loan's lifetime beyond the
call.

### D16 — Trait objects borrow through a fat pointer

`dyn Trait` is unsized: every value has an unknown concrete layout, so it is
never stored, moved, or returned by value (D10). `*dyn Trait` and
`*mut dyn Trait` are `Copy` references of two words, `{ data, vtable }`. The
reference is a borrowed view exactly like any other: coercing `*S` records the
same loan as the source, and copying or reborrowing the fat pointer extends that
loan without touching the value. Coercion preserves the reference kind, so
`*mut S` becomes only `*mut dyn Trait`; sharing it requires the explicit
`&*pointer` reborrow.

The vtable is a constant emitted once per `(concrete type, trait reference)`
pair. Its first three slots are a no-op drop function, size, and alignment,
followed by method pointers in the principal trait's declaration order and then
supertraits depth-first, deduplicated by method identity. The drop slot is a
compiler-generated no-op because no current type has a destructor; reserving it
keeps the layout stable when `Drop` lands. Size and alignment are not read by a
current operation. The layout is compiler-private and is not a stable external
ABI.

A dynamic call does not discover the concrete type at runtime. Type checking
selects the vtable slot, monomorphization collects the concrete method instance,
and the backend loads the pointer from the vtable second word and calls it
indirectly. The extracted data pointer is passed according to the selected
method's receiver form: `self: *Self` receives `*T`, while `self: *mut Self`
receives `*mut T`; a shared receiver reached through an exclusive object is
reborrowed as shared, so the callee cannot write while that loan is live.
Calling an exclusive method through `*dyn Trait` is `E3029`.

The coercion neither allocates nor copies the concrete value, and the vtable is
not copied into the value or the stack. A function reachable through a vtable is
excluded from `noalias`: its indirect call sites never passed through the static
whole-program proof, so the attribute would make the contract unverifiable.

## Type-system encoding

`seal-ty` encodes these decisions instead of leaving them to prose:

- `TyKind` has `Bool`, `Char`, `Str`, `Slice { element }`, `Int`, `Uint`,
  `Float`, `Ref { kind: RefKind, pointee }`, `Array { element, length }`,
  `Adt { adt, args }`, `Dyn { trait_def, args }`, `Tuple(TupleId)`,
  `Param(ParamTyId)`, `Unit`, `Never`, `Error`, and `Infer`.
  `AdtId` always names the declaration and `args` is the interned, structural
  list of type arguments; `Param` is a rigid parameter of one generic body.
  `Dyn` names one trait declaration plus its fully substituted arguments and is
  always the pointee of a reference.
  Tuple IDs come from a structural interner, so equal element lists share one
  `TupleId`. `RefKind` is `Shared`
  (`*T`) or `Exclusive` (`*mut T`); the kind is part of the type, so the two
  never unify. There is no nullable reference kind, because nullability is not a
  type.
- Struct declarations live in a `TyCtxt` arena as `AdtDef`/`FieldDef` records,
  so `AdtId` identifies a nominal declaration while the definition holds the
  ordered fields, their `DefId`s, names, and types. Tuples live in a separate
  structural arena keyed by their ordered `TyId`s, with no names or `DefId`s.
- `TyCtxt::is_copy`, `is_sized`, `needs_drop`, and `contains_reference` walk
  aggregate fields, tuple elements, and array elements with a cycle guard; no
  query can loop on a malformed declaration. `Str`, `Slice`, and `Dyn` are never
  `Copy` or `Sized`; a reference to any of them is both. On a declared parameter
  the structural queries are conservative: `is_copy` is false without a `Copy`
  bound, `is_sized` is true, `needs_drop` is false, and both reference
  predicates are true. The environment-aware `is_copy_in` answers from the
  body's bounds instead.
- `SubstsId` is an interned ordered argument list. `substitute(ty, args)`
  rewrites parameters through references, arrays, slices, tuples, and aggregate
  arguments without expanding a recursive definition. `Instance { def, args }`
  is the monomorphization key, and `MonoMir` bodies contain no `Param`, no
  `Infer`, and no symbolic trait call.
- `Str` and `Slice` are distinct kinds, not aliases for `*u8`, because they are
  unsized and their references carry a length (D7, D14).
- `Copy`, `Sized`, and `Drop` are **queries** (`TyCtxt::is_copy`,
  `is_sized`, `needs_drop`), not flags stored in the type. They are derived from
  the type's structure, so they cannot disagree with it. Struct values are not
  `Copy` in v0.1 (D2), so the query returns `false` for `TyKind::Adt`.
- `seal_ty::InitPaths` is the shared definite-initialization lattice; type
  checking keys it by `DefId`, borrow checking by MIR `LocalId`, and both query
  the same nearest-entry state over paths of struct fields, tuple elements, and
  constant array indices.
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
  every field projection, checks object safety when a `dyn Trait` is formed,
  records each implicit trait-object coercion with its source and target types,
  runs definite-initialization analysis over move paths, and inserts drop
  obligations.
- **Borrow checking** runs on MIR, where every move, borrow, and drop is an
  explicit statement in a control-flow graph; regions are inferred from the
  liveness of those statements, and place conflicts compare path prefixes so
  field borrows are as precise as the moves they guard.
- **Monomorphization** collects every reachable `(definition, arguments)` pair,
  substitutes the body, resolves a trait-bound call to the unique impl method or
  its inherited default, and validates that no parameter or inference variable
  survives. It also
  collects one vtable per reachable `(concrete type, trait reference)` pair and
  every method instance in its slots. It is the only phase that turns a generic
  body into concrete code.
- **Codegen** may assume that no safe reference is null or dangling and that a
  `Copy` value has no destructor. It emits each collected vtable as a constant
  with the fixed header and method slots, constructs trait-object pointers by
  copying the data address and the table address, and lowers a dynamic call as
  an indirect call through the loaded slot. It marks an exclusive-reference
  parameter `noalias` only for a function whose body never reaches storage
  through an unknown pointer or a dynamic call and whose every call site proves
  its reference arguments are disjoint when one is exclusive; otherwise two
  `*mut T` parameters may alias, and a vtable-reachable method stays unmarked.

## Invariants

1. No implicit deep copy and no implicit allocation.
2. Every live `*T` is non-null and points to initialized, still-live memory.
3. A moved-from place is never read, reassigned as a read, or dropped.
4. Every initialized owner is dropped exactly once on every exit path.
5. `Copy` and `Drop` never hold for the same type.
6. A `Sized` value never contains an unsized value directly.
7. Type layout and addresses never affect the meaning of safe programs.
8. A struct or named variant literal initializes every field exactly once;
   unknown, missing, and duplicate fields are rejected (`E2006`–`E2009`), and a
   positional variant receives exactly one argument per field in declaration
   order (`E2038`).
9. A struct or enum value is never `Copy` in v0.1; moving one field out leaves
   its siblings initialized and usable. A tuple is `Copy` exactly when every
   element is, and moving one element leaves its siblings initialized.
10. A struct or enum cannot contain itself by value (`E3016`), and every field
    and tuple element is `Sized` (`E3010`). An enum is sized when every field of
    every variant is; a tuple when every element is.
11. Layout is deterministic: declaration order, natural alignment, no
    reordering, so offsets are stable across phases and targets with the same
    data model. An enum adds a tag followed by an aligned payload, and the tag
    is the smallest unsigned integer that addresses every variant.
12. `match` binds each name once (`E2040`), or-pattern alternatives bind the
    same names (`E2041`), and a guard reads only `Copy` bindings (`E2045`). A
    guarded arm never contributes coverage, every match is exhaustive
    (`E3032`), and no arm is unreachable (`E3033`). Matching moves the
    scrutinee; a non-`Copy` field binding moves that field out of the value.
13. `dyn Trait` is unsized and never a value. `*dyn Trait` and `*mut dyn Trait`
    are exactly two pointer-sized words, data first and vtable second, and the
    reference kind is preserved by every coercion and reborrow.
14. A dynamic call's recorded principal trait reference matches the receiver's
    pointee, and its slot is the same index everywhere in the compiler.
15. Every vtable method slot has one collected concrete instance and no null
    entry. Constructing or copying a trait-object pointer allocates nothing and
    copies no concrete value.
16. An explicit numeric cast reads its operand exactly once, creates no loan,
    and writes only its destination. Float-to-integer conversion is saturated
    and maps NaN to zero, so no source value can invoke undefined behavior.
16. A function reachable through a vtable is excluded from `noalias` unless a
    future whole-program proof covers indirect calls.

## Reserved syntax

These are layers above v0.1, not gaps in it: each has its semantics defined
above and only waits for the surrounding features.

- Visibility and `Drop` for aggregates — layers on the nominal struct, enum,
  and array foundation, not redesigns of it.
- Pattern layers above v0.1 — ranges (`0..=9`), `@` bindings, and destructuring
  a bare struct — extend the same pattern tree and the same usefulness check;
  the tag and payload layout above does not change.
- User-defined implementations, trait implementations, and intrinsic methods
  share the static lookup and receiver-borrow model that generics parameterize;
  trait objects add the dynamic path in D16 without changing that model.
- Raw pointer type — nullable, arithmetic, provenance rules from RFC 3559, only
  usable inside `unsafe`.
- Owned heap buffer and allocator API — D7's owning type.

## References

- Rust Compiler Development Guide, *The ty module: representing types* and
  *Memory management in rustc* (interning, arenas, canonical types).
- Rust Compiler Development Guide, *The borrow checker* and *Region inference
  (NLL)* (liveness-based regions, outlives constraints).
- RFC 3559, *Rust has provenance*.
- Villani et al., *Tree Borrows* (aliasing model for unsafe code).
- The Rust Reference, drop order and move semantics.
