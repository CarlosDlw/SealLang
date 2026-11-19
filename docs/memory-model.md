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
aggregate that owns its fields; it is `Copy` only after an explicit `impl Copy`
covering the declaration, and then only when every field is `Copy`. Without that
implementation every whole-value transfer is a visible move. A tuple is a structural
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

### D2 — `Copy` is opt-in for nominal aggregates

Copied bitwise: all integer and float types, `bool`, `char`, `()`, `!`,
`*T`/`*mut T`, `*raw T`/`*raw mut T`, `*dyn Trait`/`*mut dyn Trait`, an array
whose element type is `Copy`, and a tuple whose elements are all `Copy`. A
struct or enum is `Copy` only after an explicit `impl Copy` that covers the
whole declaration and only when every field of every variant is `Copy`. Not
copied: `str`, slices, and `dyn Trait` (unsized), and every nominal aggregate
without that implementation, even one whose fields are all `Copy`.

The property is duplicated bitwise, with no allocation and no user-defined copy
hook; field reads still copy `Copy` fields individually. `Copy` and `Drop` are
mutually exclusive. Copying an aggregate duplicates the checked loans of its
reference fields, so the pointee stays borrowed while the copy is live; a raw
pointer field is only an address value and creates no checked loan. Arrays and
tuples need no opt-in because their elements already decide whether a bitwise
copy is safe.

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
`E3018`, because moving it would leave the owner's storage without a value.
Copying an aggregate that contains a reference copies those reference values
and keeps their checked loans live with the copy.

Raw pointers are a separate type with separate syntax: `*raw T` is nullable,
copyable, supports arithmetic, and is read-only through `*raw T`; `*raw mut T`
permits writes. A raw pointer carries provenance rather than a checked loan, so
using one is an unsafe contract (D21, D22). References remain the safe default
and never unify with raw pointers.

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

The runtime aliasing model for unsafe code is **Tree Borrows** with pointer
provenance (RFC 3559): every pointer carries the allocation and the permission
it was derived from, and accesses outside that permission are undefined
behavior. Tree Borrows accepts more real-world patterns than Stacked Borrows and
is the model Miri uses for new work. Safe references are checked statically;
raw pointers opt into this runtime contract inside `unsafe`.

### D7 — Allocation is explicit; no GC

There is no implicit heap. String literals are `*str` fat pointers (data and
length) into static read-only memory, so `str` is an **unsized pointee** and is
only usable behind a reference. The runtime provides the internal allocation
substrate (`seal_alloc`, `seal_alloc_zeroed`, `seal_realloc`, `seal_dealloc`)
and reports failure as `NULL`. Owned heap data arrives with the allocator API as
a distinct owning type that implements `Drop`; allocation failure there is
returned as a value, not hidden behind an abort. Reallocation and growth are
explicit operations of that type, never implicit on assignment. A trait object
also does
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

Equality compares values, not addresses in safe code. Padding bytes are not
observable. There is no guarantee that a local is zero-initialized. Two equal
values are interchangeable in safe code, which is what allows moves and copies
to be reordered. `==` and `!=` read their operands without moving them, and a
thin reference has no equality at all, because only its address could differ.
A raw pointer is the explicit exception: its equality compares the data address,
not the pointee or the metadata word, and it is only one of the operations that
requires the programmer to accept the unsafe contract (D21, D22).

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
pair. Its first three slots are the concrete type's drop glue, size, and
alignment, followed by method pointers in the principal trait's declaration
order and then supertraits depth-first, deduplicated by method identity. The
drop slot names one internal `void (ptr)` glue per type: it runs the declared
destructor and then drops the struct fields in declaration order, the active
enum variant, or every array element, and it is empty for a type with no
destructor. Size and alignment are not read by a current operation. The layout
is compiler-private and is not a stable external ABI.

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

### D17 — Protocol calls borrow storage and move values normally

The iteration and indexing protocols add no storage of their own. A custom
`Index::index` or `IndexMut::index_mut` returns a reference into the base, so
the selected element is the existing storage reached through an ordinary
reference; the call transfers the base loan exactly like any other
pointer-returning method. Read indexing keeps a shared loan, while writes,
compound assignments, and exclusive borrows take an exclusive loan through
`IndexMut` and conflict with live aliases under the usual rules. The compiler
adds no bounds check for a custom implementation, so the implementation must
return a valid reference for every accepted key.

A custom `for` loop evaluates the source once and stores it for the loop's
lifetime. `IntoIterator::iter` may move the source or receive a shared or
exclusive reference selected by its implementation; the returned cursor is a
local with the same storage and drop rules as any other value. `has_next`
borrows the cursor shared, `next` borrows it exclusively, and the returned item
is moved into the loop binding. A non-`Copy` item is therefore consumed once,
while the cursor and its source remain distinct values. Nothing is allocated by
the protocol itself, and no hidden heap cursor exists.

`Index` and `IndexMut` are object-safe because their methods use reference
receivers and do not mention `Self` in the result; `*dyn Index` stores the
ordinary `{ data, vtable }` pair and dispatches through the existing vtable
slot. `IntoIterator::iter` takes `self` by value, so `dyn IntoIterator` is not
dyn-compatible and is rejected at object-safety checking before any storage or
vtable is created. Built-in arrays, slices, and ranges never use these
protocols, which is why their bounds checks and representation remain
unchanged.

### D18 — Global storage has two classes

A `const` is a value with no storage. A place rooted at a const is a
**value-backed place**: reading it produces the constant value afresh, projecting
a field or element reads that part of the value, and `&CONST` materializes one
compiler-owned allocation with program lifetime. A const root is never writable
and is always initialized; assigning to it is rejected at type checking.

A `static` is one allocation with program lifetime, real storage for the borrow
checker and code generation. An immutable static is read-only after compile-time
initialization; a `static mut` is writable only inside an `unsafe` block, and
its reference may not escape the function that created it. Moving a non-`Copy`
value out of any static is rejected, because the static would be left without a
value for the rest of the program. Static roots are always initialized and are
never dropped. A block-scope static is one allocation per declaration, shared by
every call and every monomorphization of the enclosing function.

The storage model is completed at emission. A `const` or `static` value is
materialized as its exact memory image with one relocation per address it holds,
so the object file already contains the finished graph and nothing runs at load
time. An immutable static, including one that holds a pointer, is read-only
after relocation; a zero-initialized `static mut` becomes a local common symbol
in BSS; and a zero-sized static still reserves one byte, so two unrelated
zero-sized statics never share an address. A static is not `unnamed_addr`, which
is what makes its address identity observable to the program.

Borrow checking keys global storage by definition, not by the place syntax that
reaches it. A loan rooted at a static or at an addressed const is an ordinary
loan whose storage is that allocation, so two loans of disjoint fields do not
conflict while a loan of the whole allocation conflicts with a loan of any
field. A const materialized into a temporary remains a program-lifetime
allocation: the temporary is recognized as the const's storage, so returning
`&CONST` is valid. A reference copied out of an evaluated constant names the
allocation recorded by constant evaluation, including references nested in
aggregates.

A reference into any static or const outlives the function that created it and
is not a dangling local. A reference into a `static mut` is different: because
later callees cannot see that global loan in their effect summaries, returning
it is rejected (`E5012`). Passing it to an ordinary function is allowed for the
duration of the call.

### D19 — Mutable globals are ordered by effect summaries

Borrow checking is intraprocedural, so a call is the one place where another
body can reach the same `static mut`. Before checking bodies, the compiler
computes for every body the statics it may read or write, keyed by static and by
the statically known field, tuple, and constant-index path below it. A runtime
index covers the whole array path. Direct accesses, borrows, and intrinsic
receivers contribute direct effects; every call unions its callees' summaries.

A trait-bound call unions every implementation that could satisfy the bound plus
a default body, and a dynamic call unions every implementation in the principal
trait and its supertraits. This is deliberately type-independent so the analysis
runs before monomorphization. Recursion is a monotone fixed point: summaries
start empty and grow until no set changes, making the result independent of
traversal order. A call to a bodyless function is treated as touching every
static; safe Seal has no such function, so this is a defensive rule.

At each call site every summarized read must not overlap a live exclusive loan
(`E5011`), and every summarized write must not overlap any live loan (`E5013`).
A static path already granted to the call as a reference argument is checked by
the ordinary argument rules and is not applied twice. Paths use prefix overlap:
a write to a field conflicts with a loan of the whole static and vice versa.

The guarantee is single-threaded. Seal has no threads: when they arrive, safe
`static mut` access must move behind atomics or locks, or gain a `Sync`-style
contract. The unsafe gate keeps that future change localized to the access rule
and this summary.

### D20 — Destructors run at scope exit, without unwinding

A value is released when the scope that owns it ends, when its function returns
early, when a loop body leaves through `break` or `continue`, and when the value
is overwritten. Locals drop in reverse declaration order, so a value defined
later is released first; parameters follow the locals of the body. A struct
runs its destructor and then drops the fields that need it, in declaration
order; an enum drops the active variant only; an array drops elements in
increasing index order. A move transfers the release to the destination and
leaves nothing behind at the source.

A value may also be released by `drop(value)`, which consumes the operand like a
move and runs the destructor at the call. A `Copy` value, a reference, or a raw
pointer is accepted and runs nothing, because no destructor exists and nothing
is owned.

This ordering is compiled, not observed: MIR drop sites are explicit statements,
drop elaboration prunes the sites a path can never reach and guards the ones that
depend on initialization with a `bool` flag, and an assignment to an initialized
place whose type needs a destructor drops the old value first
(`DropKind::Overwrite`).

A `static` is initialized before the program starts and is never released, so
its destructor never runs. Constant evaluation has no scope to leave either: a
constant initializer or `const func` that drops a value declaring a destructor
is `E4028`, while constructing the value and storing or returning it is allowed.

Seal has no unwinding. A panic ends the program with status 101, so destructors
that would have run during a stack unwind do not run. The decision keeps the
release order purely lexical and makes every drop site statically visible to the
borrow checker.

### D21 — Raw pointers carry provenance, not a checked loan

`*raw T` and `*raw mut T` are nullable, `Copy`, `Sized` values with an explicit
provenance. A pointer formed from `&place` or `&mut place` names the same
allocation as the reference and records a byte offset and, for `str`, a slice,
or a trait object, its metadata word. `usize`/`isize` may be cast to and from a
thin pointer, but an integer-built pointer has no known allocation: it is valid
for nullness and equality, and any access through it is outside the language
contract. `with_addr` replaces the address while preserving metadata;
`expose_addr` is the deliberate address observation.

Non-wrapping arithmetic (`add`, `sub`, `offset`, `byte_add`, `byte_offset`) is
an inbounds operation: the result must stay within the allocation or at its
one-past end, and the evaluator/code generator may use an inbounds GEP. The
`wrapping_*` family may leave the allocation and is safe to compute, but an
access through a pointer that no longer names valid storage is still undefined.
`offset_from` requires both pointers to belong to the same allocation; a zero
sized pointee has distance zero. `copy` permits overlap and has `memmove`
semantics; `copy_nonoverlapping` requires disjoint regions and has `memcpy`
semantics. Writing through a raw pointer does not drop the old value.

Raw pointer equality compares the data address only. A null pointer is written
`0usize as *raw T`, and `is_null` inspects the data word. The metadata of a fat
pointer is never part of equality and cannot be forged through a thin cast.

Constant evaluation follows the same model over compiler-owned allocations.
`p as usize`, `addr`, and `expose_addr` are not deterministic before linking and
are `E4030`; writes are `E4031`; a null, misaligned, out-of-allocation, or
uninitialized read is `E4032`. Reading a static through a raw pointer in a
constant is allowed because its allocation and bytes are known at compile time.

### D22 — `unsafe` is a narrow, explicit contract

`unsafe { ... }` unlocks exactly the operations whose precondition the compiler
cannot prove: raw dereference and projection, raw writes and copies,
non-wrapping raw arithmetic and `offset_from`, mutable-static access, and calls
to `unsafe func`. It does not relax type checking, initialization, move rules,
borrow checking, or the global effect analysis. A type error inside `unsafe` is
still a type error, and a live exclusive loan still conflicts.

An `unsafe func` declaration is a caller contract, not an implicit unsafe body:
the body must open its own `unsafe { ... }` around the unchecked operations.
The marker is checked at every call site, and a destructor or the entry point
cannot be unsafe. `unsafe` has no runtime representation and no cost; it is a
static proof obligation recorded at the operation and at the call boundary.

### D23 — `Copy` is opt-in for nominal aggregates

A struct or enum becomes `Copy` only through `impl Copy for Type {}`, or the
generic form `impl<T: Copy> Copy for Wrapper<T> {}`. The implementation must
repeat the declaration's parameters in order, may not specialize an
instantiation, and there may be only one implementation for a declaration.
Every field of every variant must be `Copy`; a type parameter needs a `Copy`
bound, while a reference or raw pointer field does not constrain its pointee.

This preserves the move-by-default model: an implementation only permits the
bitwise duplication that every field already supports, and does not introduce a
deep copy, allocation, or copy hook. Alternatives rejected: auto-Copy for every
aggregate (hides duplication and makes adding a non-`Copy` field a semantic
change), Swift-style default copying (same hidden cost), and an opt-out marker
(a type would become `Copy` merely by forgetting a declaration). `Copy` and
`Drop` remain mutually exclusive, and `Copy` never changes layout, ABI, or the
value's runtime representation.

## Type-system encoding

`seal-ty` encodes these decisions instead of leaving them to prose:

- `TyKind` has `Bool`, `Char`, `Str`, `Slice { element }`, `Int`, `Uint`,
  `Float`, `Ref { kind: RefKind, pointee }`, `RawPtr { kind: RawPtrKind, pointee }`,
  `Array { element, length }`, `Adt { adt, args }`, `Dyn { trait_def, args }`,
  `Tuple(TupleId)`, `Param(ParamTyId)`, `Unit`, `Never`, `Error`, and `Infer`.
  `AdtId` always names the declaration and `args` is the interned, structural
  list of type arguments; `Param` is a rigid parameter of one generic body.
  `Dyn` names one trait declaration plus its fully substituted arguments and is
  always the pointee of a reference.
  Tuple IDs come from a structural interner, so equal element lists share one
  `TupleId`. `RefKind` is `Shared`
  (`*T`) or `Exclusive` (`*mut T`); the kind is part of the type, so the two
  never unify. `RawPtrKind` is `Const` (`*raw T`) or `Mut` (`*raw mut T`), and
  its kind is also part of the type. There is no nullable reference kind,
  because nullability is not a reference; nullability belongs to raw pointers.
- Struct declarations live in a `TyCtxt` arena as `AdtDef`/`FieldDef` records,
  so `AdtId` identifies a nominal declaration while the definition holds the
  ordered fields, their `DefId`s, names, and types. Tuples live in a separate
  structural arena keyed by their ordered `TyId`s, with no names or `DefId`s.
- `TyCtxt::is_copy`/`is_copy_with`, `is_sized`, `needs_drop`, and
  `contains_reference` walk aggregate fields, tuple elements, and array
  elements with a cycle guard; no query can loop on a malformed declaration.
  `Copy` combines the built-in structural cases with the validated `CopyImpls`
  registry for nominal ADTs, so an `Adt` is `Copy` only when its implementation
  was registered and every field of every variant passes the same query.
  `Str`, `Slice`, and `Dyn` are never `Copy` or `Sized`; a reference to any of
  them is both. On a declared parameter the structural queries are
  conservative: `is_copy` is false without a `Copy` bound, `is_sized` is true,
  `needs_drop` is false, and both reference predicates are true. The
  environment-aware `is_copy_with` answers from the body's bounds instead.
- `SubstsId` is an interned ordered argument list. `substitute(ty, args)`
  rewrites parameters through references, arrays, slices, tuples, and aggregate
  arguments without expanding a recursive definition. `Instance { def, args }`
  is the monomorphization key, and `MonoMir` bodies contain no `Param`, no
  `Infer`, and no symbolic trait call.
- `Str` and `Slice` are distinct kinds, not aliases for `*u8`, because they are
  unsized and their references carry a length (D7, D14).
- `Copy`, `Sized`, and `Drop` are **queries** (`TyCtxt::is_copy`,
  `is_copy_with`, `is_sized`, `needs_drop`), not flags stored in the type.
  `Copy` is derived from the built-in type shape and, for an `Adt`, from the
  validated implementation registry recorded by HIR lowering; `Sized` and
  `Drop` remain structural. The answer for an `Adt` is field-recursive and
  cycle-safe, so it cannot disagree with the declaration.
- `seal_ty::InitPaths` is the shared definite-initialization lattice; type
  checking keys it by `DefId`, and MIR keys it by `LocalId` for borrow checking
  and drop elaboration, so every phase queries the same nearest-entry state over
  paths of struct fields, tuple elements, and constant array indices.
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
  obligations for the destructor HIR recorded from a validated `impl Drop`.
- **Drop elaboration** runs on MIR before constant evaluation and borrow
  checking. One drop site expands into the values the type's glue visits: the
  fields of a struct without a destructor, in declaration order, and one unit for
  an enum, an array, or a type with a declared destructor. A site whose value is
  definitely gone is removed instead of guarded, which is what keeps a constant
  from reporting a destructor it never runs. A site that depends on the path is
  guarded by a `bool` flag that an assignment sets and a move or a drop clears,
  and an assignment to an initialized place whose type needs a destructor drops
  the old value first (`DropKind::Overwrite`).
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
4. Every initialized owner is dropped exactly once on every exit path. A drop
   site whose value is definitely gone is removed instead of guarded, a
   conditional site tests a `bool` flag that assignments set and moves and drops
   clear, and an assignment to an initialized place whose type needs a
   destructor drops the old value first.
5. `Copy` and `Drop` never hold for the same type.
6. A `Sized` value never contains an unsized value directly.
7. Type layout and addresses never affect the meaning of safe programs.
8. A struct or named variant literal initializes every field exactly once;
   unknown, missing, and duplicate fields are rejected (`E2006`–`E2009`), and a
   positional variant receives exactly one argument per field in declaration
   order (`E2038`).
9. A struct or enum value is `Copy` only after an explicit implementation that
   covers the declaration and every field of every variant; moving one field out
   of a non-`Copy` aggregate leaves its siblings initialized and usable. A tuple
   is `Copy` exactly when every element is, and moving one element leaves its
   siblings initialized.
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
17. A function reachable through a vtable is excluded from `noalias` unless a
    future whole-program proof covers indirect calls.
18. A place root is a binding, a const, or a static. Only a binding root takes
    part in definite initialization; const and static roots are always
    initialized, and const roots are never writable.
19. A `static mut` place is accessed only inside an `unsafe` block, and only in
    ordinary run-time code; a constant context rejects every mutable-static
    access. An immutable static is never written or borrowed exclusively.
20. A non-`Copy` value is never moved out of a static. A const has no storage,
    so reading it by value always produces a fresh value.
21. A `const` or `static` initializer and a `const func` body run at compile
    time: they cannot call non-`const` functions, use dynamic dispatch or custom
    protocols, or carry `*mut T`.
22. Every reference produced by constant evaluation names an allocation owned by
    the compiler, with an offset inside that allocation and the referenced type.
    A static owns exactly one allocation for the whole program; a promoted
    temporary and an addressed const each own a program-lifetime allocation.
23. A reference to a const-fn frame allocation never survives that frame
    (`E4013`); promotion is the only way for a temporary's address to escape.
24. A loan rooted at a static, an addressed const, or a materialized const
    temporary names that program-lifetime allocation. Disjoint fields stay
    disjoint, and a whole-allocation loan conflicts with every field loan.
25. A reference into a static or const may be returned without being treated as
    a dangling local. A reference into a `static mut` may not escape the
    function that created it (`E5012`).
26. Every call site applies the callee's transitive mutable-static effects: a
    read conflicts with a live exclusive loan (`E5011`) and a write conflicts
    with any live loan (`E5013`). An effect already covered by a reference
    argument is checked only by the ordinary argument rules.
27. Constant evaluation is deterministic: allocation order, dependency order,
    step counting, and diagnostics do not depend on hash iteration. A value
    dependency cycle is reported (`E4007`), while an address-only cycle is
    accepted because every static allocation exists before evaluation.
28. A raw pointer is `Copy`, `Sized`, and destructor-free. `*raw T` and
    `*raw mut T` are distinct types, and neither unifies with a reference.
    A sized pointee gives one word; `str`, a slice, or a trait object gives a
    two-word `{ data, metadata }` value.
29. A cast from a reference may only preserve or discard permission; it may not
    add mutability. Raw-to-raw casts preserve or change metadata compatibly, and
    only `usize`/`isize` cross the pointer/integer boundary. A fat pointer is
    never forged from a thin pointer or an integer.
30. Raw dereference, writes, copies, non-wrapping arithmetic, `offset_from`,
    mutable-static access, and calls to `unsafe func` require an `unsafe` block;
    safe queries do not. `unsafe` does not disable any other check.
31. A raw access carries allocation provenance. Internal field, index, and
    pointer offsets use GEP; `ptrtoint`/`inttoptr` appear only for the explicit
    `addr`, `expose_addr`, `with_addr`, address casts, equality, and
    `offset_from` observations. The metadata word of a fat pointer is preserved
    by `with_addr` and compatible raw-to-raw casts, and discarded only by
    `data()`.
32. Constant evaluation accepts raw pointers over compiler-owned allocations.
    It rejects an unknown address (`E4030`), a write (`E4031`), and an invalid
    read (`E4032`); a null or integer-built pointer can still be queried,
    compared, and stored.
33. A function whose body may reach storage through a raw pointer, an unsafe
    call, a dynamic call, or an unknown pointer is excluded from `noalias`.
    The attribute remains a proven optimization, never a promise that unsafe
    code must uphold.

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
- Owned heap buffer and allocator API — D7's owning type.

## References

- Rust Compiler Development Guide, *The ty module: representing types* and
  *Memory management in rustc* (interning, arenas, canonical types).
- Rust Compiler Development Guide, *The borrow checker* and *Region inference
  (NLL)* (liveness-based regions, outlives constraints).
- RFC 3559, *Rust has provenance*.
- Villani et al., *Tree Borrows* (aliasing model for unsafe code).
- The Rust Reference, drop order and move semantics.
