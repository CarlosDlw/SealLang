# Seal language overview

Status: implemented in the current language version (`0.1`).

Seal is a small systems language for programs whose memory layout, runtime cost,
and failure modes are part of the design. It gives safe code static ownership,
non-null references, deterministic destruction, and no managed runtime, while
keeping an explicit `unsafe` layer for the operations a compiler cannot prove.

This chapter states the philosophy, goals, and design principles behind that
surface. It is the map, not the statute: every rule below is specified in a
chapter or reference document, and those documents win when a summary sentence
could be read two ways.

## Philosophy

**Cost is part of the interface.** A language construct should not allocate,
copy a deep structure, take a lock, or do work the source does not imply.
Bounds checks and contract panics are part of the operation the source wrote;
hidden allocation, deep copies, and unnamed runtime dispatch are not.
Performance is not a property added by the optimizer later; it is a consequence
of the operations the programmer chose.

**Safety is a proof, not a service.** The compiler proves what it can before the
program runs. There is no garbage collector, reference counter, tracing runtime,
or dynamic borrow flag to catch what the type system missed. A safe program that
type-checks and borrow-checks does not rely on a runtime service to reclaim
memory or to track borrows.

**Ownership is the model, not a lint.** Every value has one owner. Transfers are
moves by default. Destruction happens at a statically known point. These are not
optional diagnostics layered over a garbage-collected or reference-counted
runtime; they are how values and storage are represented.

**Local reasoning beats global cleverness.** A function signature, a type, and a
borrow should be enough to understand a call. Seal avoids whole-program
inference, hidden specialization, ambient reflection, and runtime dispatch that
the source does not name. What cannot be proved locally is either rejected or
made an explicit contract.

**Unsafe must be a contract, not a mode.** `unsafe` unlocks a closed list of
operations with preconditions the compiler cannot check. It does not turn off
type checking, initialization, moves, or borrow checking, and it never weakens
the guarantees of safe code that calls it.

**The toolchain is part of the language.** Stable error codes, `seal explain`,
deterministic builds, lockfiles, API snapshots, and reproducible artifacts are
not packaging afterthoughts. A language that cannot explain a failure or
reproduce a binary is not ready for production.

## Goals

| Goal | What it means | Where the design enforces it |
|------|---------------|------------------------------|
| **Memory safety without a managed runtime** | No use-after-free, dangling reference, null reference, double drop, uninitialized read, or use-after-move in safe code; no GC or reference counting | Type checking, definite initialization, borrow checking, drop elaboration |
| **Predictable cost** | No implicit heap allocation, deep copy, synchronization, boxing, or hidden dispatch | Moves, opt-in `Copy`, inline closures, static trait calls, explicit numeric casts |
| **Deterministic lifetime** | Every scope-owned value is released exactly once on every normal exit path, in a defined order, unless it is deliberately forgotten | `Drop`, lexical drop scopes, guarded MIR drop sites, `mem::forget`, no unwinding |
| **No lifetime annotations** | Regions are inferred from liveness; the source never writes region parameters | Borrow checker over MIR, outlives constraints, diagnostics at the conflicting use |
| **Explicit unsafe and foreign boundaries** | Unchecked operations and C calls state their contract at the operation and at the call site | `unsafe` blocks, `unsafe func`, `extern "c"`, raw pointers with provenance |
| **Deterministic layout and builds** | Field order, enum layout, plans, lockfiles, and artifacts are stable for the same inputs | `seal-layout`, sorted project resolution, content-addressed action graph, `seal verify-repro` |
| **Stable diagnostics and compatibility** | Error codes do not get reused; public API changes are classified; tests and docs share one model | `seal-error-codes`, `seal-api` snapshots, `seal metadata`, `docs/project/acceptance.md` |
| **Room to grow without redesign** | New layers extend existing phases instead of adding a second type system or a parallel pipeline | Typed HIR/MIR boundaries, interned types, monomorphization, closed intrinsic catalog |

These goals constrain each other. For example, "predictable cost" rules out a
copy-on-write string, and "deterministic lifetime" rules out exceptions that
unwind through frames. Safety is a precondition rather than a tradeable
feature: a faster or more convenient design that weakens a safe-code guarantee
is rejected. Within safety, determinism and visible cost take precedence over
convenience that hides work. When goals conflict, Seal chooses the one that
keeps the program's behavior visible in the source.

## The model in one page

| Question | Seal's answer |
|----------|---------------|
| What does assignment do? | Moves the value, unless its type is `Copy`, in which case it duplicates the bits |
| Who releases a value? | Its owner, when the owning scope ends; fields in declaration order, locals in reverse declaration order. `mem::forget` instead transfers it to no owner and leaks it |
| Can a reference be null? | No. Nullability belongs to `*raw T`/`*raw mut T`, which require `unsafe` to dereference |
| How is aliasing controlled? | Shared XOR exclusive: many `*T` or one `*mut T`; the kind is part of the type |
| When is memory initialized? | Definite-initialization analysis tracks each move path; reading an uninitialized path is an error |
| What happens on a panic? | A hosted program terminates with status 101 and a diagnostic; a freestanding target traps; nothing unwinds |
| What runs at compile time? | `const` initializers and `const func` bodies, within constant-evaluation limits |
| What is layout-stable? | Declaration order, natural alignment, enum tag/payload rules, and `extern struct` C layout; internal layout is not an external ABI |
| What must `unsafe` justify? | Raw accesses, mutable statics, non-wrapping raw arithmetic, and calls to `unsafe func` |
| What happens at a C boundary? | Ownership follows C's contract; Seal does not infer drop, aliasing, or unwinding promises across it |

The smallest complete program still shows the central idea:

```seal
struct Point {
  x: i32,
  y: i32,
}

// `*Point` is a shared, non-null reference. The callee can read the point but
// cannot mutate it, and the loan is checked at the call site.
func sum(point: *Point) -> i32 {
  (*point).x + (*point).y
}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  sum(&point)
}
```

`point` owns the struct. `&point` creates a shared loan for the duration of the
call. `sum` reads through the reference without copying the struct or allocating
anything. The program returns `42`.

## Design principles

### 1. Move by default; copying is opt-in

Assignment, argument passing, and return move a value. A move transfers the
destruction obligation and leaves the source unusable. Scalars, references, raw
pointers, arrays, and tuples are `Copy` structurally; a struct or enum becomes
`Copy` only through an explicit `impl Copy`, and only when every field of every
variant is `Copy`.

The opt-in rule is deliberate. Adding a non-`Copy` field to a type that silently
copied would otherwise change the meaning of existing programs. Requiring the
implementation makes that change visible, keeps `Copy` and `Drop` disjoint, and
ensures a copy is always a bitwise duplication rather than a hidden deep copy or
user hook.

### 2. Shared XOR exclusive, with inferred regions

A place is either shared-borrowed any number of times or exclusively borrowed
once. `*T` is shared and read-only; `*mut T` is exclusive and read-write. The
two never unify, and the borrow checker rejects a use that overlaps a live loan
of the same storage, except for the reborrows a loan authorizes. Loans are
sliced by place prefix, so borrowing `p.x` leaves `p.y` usable.

There are no lifetime parameters. Each reference gets a region from the program
points where it may still be used, closed under outlives constraints. A returned
reference must be backed by a region that outlives the call. This keeps
signatures readable and makes the checker's reasoning local, at the cost of
being conservative where the target of a reference is not statically known.

### 3. Initialization is a tracked state

`let x: T;` creates uninitialized storage. Reading it before assignment is an
error; assignment initializes it; a move makes it uninitialized again. The
analysis is per move path, so fields, tuple elements, and constant array indices
initialize independently, while a runtime index conservatively requires the
whole array.

The same lattice is shared by type checking, MIR, borrow checking, and drop
elaboration. A value cannot be read before it exists, and a destructor cannot
run on a value that was never initialized.

### 4. Destruction is lexical, and panics do not unwind

A value is released when its scope ends, when control leaves through `return`,
`break`, or `continue`, and when it is overwritten. Locals drop in reverse
declaration order; parameters follow the body's locals; struct fields drop in
declaration order; an enum drops only its active variant; an array drops its
elements in increasing index order. `drop(value)` consumes a value and releases
it early. `mem::forget(value)` is the explicit safe exception: it consumes the
value and runs no destructor, so the value's resources leak on purpose.

Seal has no exceptions and no unwinding. On a hosted target, a panic writes a
diagnostic and ends the process with status 101; the `panic(message)` built-in
uses the same path, and `exit(code)` ends the process with the supplied status.
A freestanding target turns a panic into a trap. Destructors that would have run
during a stack unwind do not run, which keeps
release order purely lexical and every drop site statically visible. Code that
must release a resource on failure uses an ordinary enum value and explicit
control flow, not a hidden unwind path.

### 5. Identity is explicit; layout is deterministic

Structs and enums are nominal: two declarations with the same fields are
different types. Tuples are structural: `(i32, bool)` is the same type wherever
it is written. Fields are laid out in declaration order with natural alignment,
without reordering. An enum stores the smallest unsigned tag that addresses its
variants, followed by an aligned payload. `extern struct` opts into the target's
C layout.

Safe code cannot observe addresses, padding, or field order to change the
meaning of a program. Deterministic layout makes artifacts reproducible and
makes the ABI reviewable; it gives up the padding optimizations that would
require reordering. The internal layout is a compiler contract, not a stable
external ABI; `extern struct`, `export "c"`, and C function signatures are the
explicit interoperability surface.

### 6. Conversions and configuration are explicit

There are no implicit numeric conversions. Integer literals take their type from
context; otherwise numeric types unify only when they are identical. Widening
and narrowing happen only through `as`, which reads its operand once, allocates
nothing, creates no loan, and never panics. Float-to-integer conversion is
saturated and maps NaN to zero.

Configuration follows the same rule. `cfg(...)` is a typed boolean expression
over features, target properties, environment variables, debug assertions, and
test/bench modes. A misspelled key is diagnosed at parse time instead of
silently evaluating to false. `cfg` selects items, modules, and imports before
name resolution; it does not silently alter a type's layout after the fact.

### 7. Unsafe is narrow, local, and provenance-aware

`unsafe { ... }` unlocks a closed list of operations: raw dereference and
projection, raw writes and copies, non-wrapping raw arithmetic, `offset_from`,
mutable-static access, and calls to `unsafe func`. It does not relax type
checking, initialization, move rules, borrow checking, or global effect
analysis. A type error inside `unsafe` is still a type error.

An `unsafe func` is a caller contract, not an unsafe body: the implementation
must open its own `unsafe` block around each unchecked operation, and every call
site must acknowledge the contract. Raw pointers carry allocation provenance;
accesses outside that provenance are undefined behavior under the Tree Borrows
model. Optimizations such as `noalias` are emitted only when the compiler has
proved them, never as a promise that unsafe code must uphold.

### 8. Boundaries are declared

Modules are namespaces, not directories. A file joins one with a first-item
`module a::b;` declaration; files without one contribute to the package root.
Imports are absolute and file-scoped; there are no relative imports and no
globs. Visibility is `private` by default, `internal` to the package, and
`public` only through the package's declared API. Symbols and fingerprints are
qualified by the package instance and module where needed, so equal names in
different packages never collide.

The C boundary is declared the same way. An `extern` declaration names storage
or code Seal does not own, every call and every external storage access is
inside `unsafe`, and ownership stays on its side: C memory is released by C,
Seal memory by Seal. A callback cannot unwind through a Seal frame, and an
external declaration receives no inferred `noalias`, `readonly`, or `nounwind`.

### 9. Determinism is a whole-pipeline property

Determinism is not limited to layout. Packages, dependency edges, lockfile
entries, graphs, and metadata are sorted before they are compared, hashed, or
printed. A build action is keyed by a closed list of inputs: source digests,
toolchain, target, profile, relevant environment, native probe results, and the
action kind. The artifact cache verifies bytes on read. `seal verify-repro`
builds the same project in two directories and compares every artifact byte for
byte.

The same principle covers the language surface: error codes are stable, test
harnesses are generated from one model, and public-API snapshots classify
changes rather than relying on a human diff. Determinism is what makes caching,
remote execution, auditing, and reproducible releases meaningful.

## Accepted trade-offs

Every design choice has a cost. These are accepted, not accidental:

| Decision | Accepted cost | Rejected alternative |
|----------|---------------|----------------------|
| Move by default; `Copy` is opt-in | An annotation on aggregates that should copy; adding a non-`Copy` field removes the implementation | Implicit deep copy, implicit reference semantics, or Swift-style default copying |
| No lifetime annotations | Conservative rejection where a reference's target is unknown or a whole-program proof is unavailable | Lexical lifetimes that reject valid programs, or runtime borrow tracking |
| No unwinding | Destructors do not run on panic; cleanup on failure is explicit | Exceptions or unwinding that make drop order dynamic |
| Safe explicit leak via `mem::forget` | A forgotten value's resources are never released; the leak is visible at the call | An unsafe cleanup escape hatch or a hidden leak in ordinary ownership |
| No implicit numeric conversions | Explicit `as` at every width or signedness change | Silent truncation or sign changes |
| File-scoped absolute imports | Repetition across fragments; no glob imports | Relative imports and globs that hide name provenance |
| Deterministic layout without reordering | Larger structs when padding could be reduced | Field reordering that changes offsets and ABI |
| No managed runtime | Ownership and lifetime are explicit; heap storage is never created implicitly | GC pauses, reference-count traffic, or a tracing runtime |

The trade-off table is also a compatibility guide: changing one of these
decisions changes the language's contract, not just an implementation detail.

## What safe code guarantees

For a program that passes all checks, and absent `unsafe` or foreign code:

- a live reference is non-null and points to initialized, still-live memory;
- a moved-from place is never read, reassigned as a read, or dropped;
- every initialized value owned by a scope is dropped exactly once on every
  normal exit path, unless it was consumed by `mem::forget`; forgetting is safe
  and leaks the value's resources;
- an exclusive loan excludes every other use of the same storage except the
  reborrows it authorizes;
- no language operation allocates, deep-copies, boxes, or synchronizes
  implicitly;
- safe integer arithmetic has defined behavior; a violated runtime contract
  such as an out-of-bounds index or a zero divisor terminates with a diagnostic
  instead of corrupting memory;
- a panic does not unwind, so no destructor observes a half-torn stack.

These are the properties the rest of the system is allowed to rely on. The
optimizer may assume them; a library may document them; an unsafe block that
breaks them is a bug in that block, not a permitted program.

## How this guide is organized

The chapters apply the principles in order, from values to projects:

| Start here | Then |
|------------|------|
| [`copy.md`](copy.md) | Moves, opt-in `Copy`, drops, and patterns |
| [`raw-pointers.md`](raw-pointers.md) | The provenance model behind `unsafe` |
| [`ffi.md`](ffi.md) | The C ownership and unwinding boundary |
| [`first-class-functions.md`](first-class-functions.md) | Inline closures, call kinds, and function pointers |
| [`modules.md`](modules.md) | Names, visibility, configuration, and package APIs |
| [`stdlib.md`](stdlib.md) | `core`/`std`, `Option`/`Result`, paths, and the prelude |
| [`testing.md`](testing.md) | Test and benchmark modifiers, harness generation, and assertions |
| [`../project/manifest.md`](../project/manifest.md) | The project and package contract |
| [`../project/builds.md`](../project/builds.md) | The deterministic action graph and cache |
| [`../project/architecture.md`](../project/architecture.md) | How the compiler and project layers fit together |
| [`../project/acceptance.md`](../project/acceptance.md) | The evidence for every guarantee above |

The normative references are [`../types.md`](../types.md),
[`../memory-model.md`](../memory-model.md),
[`../architecture.md`](../architecture.md), and
[`../codegen.md`](../codegen.md). The guide chapters explain the surface; the
references define the invariants the compiler is built to preserve.

## Extending the language

A feature is not part of Seal until it answers all of these questions:

1. **Ownership:** does it move, copy, borrow, or create an owner?
2. **Aliasing:** what loan does it create, and what does it conflict with?
3. **Initialization and destruction:** when does its storage become valid, and
   when is it released?
4. **Layout and ABI:** does it change size, alignment, field order, symbols, or
   the C surface?
5. **Constant evaluation:** can it run at compile time, and what is rejected?
6. **Failure:** does it return a value, panic, or require `unsafe`?
7. **Diagnostics:** which stable code reports each violation, and what does
   `seal explain` say?
8. **Tooling:** how do plans, cache keys, metadata, API snapshots, tests, and
   docs represent it?

A feature that cannot answer those questions is not ready, even if it compiles.
A feature that answers them by weakening an invariant above is a new language
version, not an extension.

The bet behind Seal is simple: if the compiler proves ownership, aliasing,
initialization, and destruction statically, the runtime does not need to guess.
What cannot be proved is spelled `unsafe`, and everything else stays visible in
the source.
