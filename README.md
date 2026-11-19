# Seal

Seal is a small systems language built around value semantics, compile-time
ownership, and zero-cost references. Programs are type-checked, borrow-checked,
and compiled to native executables through LLVM. The compiler is written in
Rust.

| Property | Seal |
|----------|------|
| Memory safety | Static borrow checking; no garbage collector, reference counting, or tracing |
| Cost model | No hidden allocation, deep copy, or synchronization |
| Lifetimes | Inferred from liveness; no lifetime syntax in the source |
| Artifacts | LLVM object, assembly, or a linked native executable |
| Diagnostics | Stable error codes with long-form explanations (`sealc explain`) |

## Quick start

```text
cargo build --release
./target/release/sealc build -g examples/006_structs.sl -o /tmp/seal-structs
/tmp/seal-structs; echo $?
```

```text
10
```

`sealc build` runs the whole pipeline: it type-checks, borrow-checks, lowers
checked MIR to LLVM IR, optimizes, and links an executable. `sealc run` builds
that executable in a temporary directory, runs it with inherited standard
streams, removes it afterwards, and returns the program's exit status. `sealc check`
stops after analysis, while `sealc build --emit=obj` and `sealc build --emit=asm`
write intermediate artifacts.

## The language at a glance

```seal
struct Point {
  x: i32,
  y: i32,
}

// `*Point` is a shared, non-null reference: read-only, copyable, and checked
// against every other loan of the pointed-to storage.
func sum(point: *Point) -> i32 {
  (*point).x + (*point).y
}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  sum(&point)
}
```

Every value has one owner. Assignment, argument passing, and return move the
value unless its type is `Copy`; a struct owns its fields and is moved as a
whole. An enum is a nominal sum type: every value is exactly one variant, either
a unit variant, a positional variant such as `Some(i32)`, or a named variant
such as `Circle { radius: i32 }`, and the layout carries a discriminant plus the
active payload. `*T` grants shared access and `*mut T` grants exclusive access, and the
two never unify. Exclusive arguments are checked at every call site (`E5009`,
`E5010`), which is what allows the backend to emit `noalias` without turning
safe code into undefined behavior.

A type may declare `impl Drop` and own the release of what it holds. The
destructor runs when the value leaves its scope, when a function returns early,
or when a loop body leaves, and fields, the active enum variant, and array
elements follow in a fixed order. `drop(value)` releases a value early and
consumes it like a move, so using it afterwards is `E5005`. A `static` is never
released, and a constant cannot run a destructor (`E4028`).

Tuples are structural product types: `(i32, bool)` is the same type wherever it
is written, and elements are accessed by position with `.0`, `.1`, and so on.
`(T,)` is a one-element tuple, `(T)` only groups, and `()` is the unit type. A
tuple is `Copy` exactly when every element is; otherwise it moves as a whole,
while a single element can still be moved, borrowed, or initialized on its own.
Tuple patterns write one subpattern per element and follow references like
struct patterns do.

`match` is an expression over enum variants, structs, tuples, arrays, and scalar
values. A braced block is an ordinary expression, so an arm body can run
statements and produce a value. Patterns nest: they match unit, positional, and
named variants, destructure structs (a trailing `..` ignores the fields an arm
does not use), destructure tuples by position, follow a reference to a struct,
variant, or tuple without an explicit dereference, match every element of a
fixed-size array or a slice, bind fields, compare integer, float, character, and
boolean literals, range over
integers and characters (`0..=9`, `'a'..='z'`), bind a whole value while narrowing it
(`name @ 0..=9`), and combine alternatives with `|`. An arm may carry an `if`
guard, which sees only `Copy` bindings because a failed guard must leave the
scrutinee available for the next arm. Coverage is proved by the usefulness
algorithm: enums and `bool` are exhaustive once every constructor is covered,
integer and character ranges partition their domain so adjacent ranges can be
exhaustive, slice patterns partition the runtime length domain the same way,
floats need a catch-all, and an arm that no value can reach is `E3033`.

`for` loops iterate an integer range (`0..n` or `0..=n`), an array by value, a
borrowed array or slice, or a custom source. A custom source resolves through
`IntoIterator<Iter, Item>` first, whose cursor must implement `Iterator<Item>`;
a source that implements `Iterator<Item>` itself uses the direct fallback. The
loop pattern must be irrefutable, and `continue` advances to the next element,
so a skipped iteration still makes progress.

Custom indexing implements `Index<Key, Value>` for reads and
`IndexMut<Key, Value>` for writes, compound assignments, and exclusive borrows.
Both protocols return references into the base, so indexing remains a place and
nests without allocating. Built-in arrays and slices keep their precedence and
bounds checks; a custom implementation receives the key type written by the
expression and selects the value type from its implementation or the expected
type.

`if let pattern = value { ... } else { ... }` evaluates the value once and runs
the body only when the pattern matches. Its pattern may be refutable, and the
bindings it introduces exist only in the body. `else if let` chains mix freely
with ordinary `else if`, and an `if let` without `else` has type `()`.

`while let pattern = value { ... }` reevaluates the value and retests the
pattern before every iteration. A successful match binds its names only in the
loop body; a failed match exits the loop. `break` and `continue` act on the
innermost loop, and `continue` reevaluates the scrutinee.

`const` declares a compile-time value with no storage of its own, and `static`
declares one allocation for the whole program. Both are initialized before the
program starts, and `const func` may run at compile time or at runtime. Only
`static mut` is writable, and every access to it needs an `unsafe { ... }`
block:

```seal
const LIMIT: i32 = 6;
static STEP: i32 = 7;
static mut CALLS: i32 = 0;

const func scaled(value: i32) -> i32 {
  value * LIMIT
}
```

## Command line

```text
sealc build [OPTIONS] <INPUT>...
sealc check [OPTIONS] <INPUT>...
sealc run [OPTIONS] <INPUT> [-- <ARGS>...]
sealc explain <CODE>
sealc emit-runtime <PATH>
sealc version
sealc help [COMMAND]
```

| Command | Effect |
|---------|--------|
| `build` | Type-check, borrow-check, and write an executable, object, or assembly |
| `check` | Run every analysis pass through monomorphization; write nothing |
| `run` | Build into a temporary executable, run it, and return its exit status |
| `explain` | Print the long-form explanation of a diagnostic code |
| `emit-runtime` | Export the embedded runtime archive |
| `version` | Print the compiler version, language version, and host triple |
| `help` | Print top-level help or the help of one command |

`build` options:

| Option | Effect |
|--------|--------|
| `--emit=obj` | Write a relocatable object |
| `--emit=asm` | Write target assembly |
| `--emit=exe` | Link a native executable (default) |
| `-O0` … `-O3`, `-O` | Select the optimization level (`-O` means 3) |
| `-g` | Embed DWARF debug information |
| `-o <PATH>` | Write the artifact to `PATH` (one input only) |
| `--runtime-archive <PATH>` | Link a different runtime archive |

`build` and `check` both accept `--target`, `--language-version`,
`--allow-unstable-features`, and the diagnostic options (`--error-format`,
`--color`, `--max-errors`, `--remap-path-prefix`, `--time-passes`).

`run` accepts the optimization, target, runtime, and diagnostic options above,
but not `--emit` or `-o`: it owns the temporary output path. It executes only
host programs, so a target with a different architecture or OS is rejected with
status `1` before compilation. Arguments after `--` are forwarded unchanged.

Without `-o`, `sealc build` writes `file`, `file.o`, or `file.s` next to the
input. The exit status is `0` on success, `1` after a compile error, `2` after a
usage error, and `101` after an internal compiler error. `sealc run` returns the
exit status of the program it executes, including `101` for a Seal runtime panic.

## Verifying a commit

`scripts/verify-commit.sh [<commit>]` checks out a commit, runs formatting, lint,
and the full test suite against it, and restores the original checkout
afterwards. The working tree must be clean. Every commit must pass this
isolation check on its own.

## Types

| Family | Types | Notes |
|--------|-------|-------|
| Signed integers | `i8` … `i128`, `isize` | Two's complement; `isize` follows the target pointer width |
| Unsigned integers | `u8` … `u128`, `usize` | Wrapping arithmetic; `usize` follows the target pointer width |
| Floating point | `f32`, `f64` | IEEE 754 binary32 and binary64 |
| Text | `bool`, `char`, `str`, `*str` | `char` is a Unicode scalar value; `str` is unsized and only usable behind a reference |
| References | `*T`, `*mut T` | Non-null, non-owning, and `Copy`; shared or exclusive |
| Tuples | `(T, U)`, `(T,)`, `()` | Structural product types; fields are `.0`, `.1`, …; `Copy` when every element is |
| Arrays | `[T; N]` | Fixed length, owns its elements, and is `Copy` when `T` is |
| Slices | `[T]`, `*[T]`, `*mut [T]` | Borrowed views over contiguous elements; references carry data and length |
| Trait objects | `dyn T`, `*dyn T`, `*mut dyn T` | Unsized pointee behind a two-word `{ data, vtable }` reference; dynamic dispatch through the vtable |
| Aggregates | `struct S { … }` | Nominal, owns its fields, and is not `Copy` |
| Enums | `enum E { … }` | Nominal sum type with unit, positional, or named variants; owns the payload and is not `Copy` |
| Methods | `impl S { … }` | Inherent methods and associated functions; static calls on concrete types, dynamic calls through trait objects |
| Traits | `trait T { … }`, `impl T for S { … }` | Methods may declare default bodies, which impls inherit or override; static dispatch on concrete types and bounds, and object-safe traits dispatch dynamically through `*dyn T` |
| Generics | `func f<T>(…)`, `struct S<T>`, `enum E<T>`, `impl<T> S<T>` | Monomorphized: one concrete body and layout per type argument; bounds are static and closed-world |

There are no implicit numeric conversions: every arithmetic and comparison
operand must already have the expected type, and an unsuffixed integer literal
defaults to `i32` while an unsuffixed float defaults to `f64`. Conversions are
explicit and numeric-only, written `value as Type`: integers narrow and extend,
integers convert to and from `f32`/`f64`, and float-to-integer truncates toward
zero, saturates, and maps NaN to zero. `bool`, `char`, enums, references, and
aggregates are not `as` casts; `bool.to_i32()` and `char.to_u32()` remain the
named conversions for those scalars.

See [`docs/types.md`](docs/types.md) for sizes, alignments, ranges, literal
syntax, casts, `Copy`/`Sized`/`Drop`, reference rules, and the layout algorithm.

## Documentation

| Document | Contents |
|----------|----------|
| [`docs/memory-model.md`](docs/memory-model.md) | Ownership, moves, borrowing, allocation, and the decisions behind them |
| [`docs/types.md`](docs/types.md) | The complete type list, properties, literals, layout, and ABI representation |
| [`docs/architecture.md`](docs/architecture.md) | Compiler phases, crate responsibilities, and public invariants |
| [`docs/codegen.md`](docs/codegen.md) | LLVM lowering, ABI, debug information, runtime, and optimization |
| [`RELEASES.md`](RELEASES.md) | Release notes and source migration for language changes |

## Status

The current language surface is functions, nominal structs and enums,
structural tuples, fixed-size arrays, slices, shared and exclusive references, `let` bindings with
definite initialization, `if`, `if let`, `while`, `while let`, `for` over integer ranges and sequences,
`break`, `continue`, `return`, `match`
expressions with structural patterns, guards, and exhaustiveness checking, the
usual arithmetic, comparison, logical, and bitwise operators, explicit numeric
casts with `as`, compiler-owned intrinsic
methods on integers, floats, `bool`, `char`, arrays, and slices, inherent `impl`
methods and associated functions on nominal types, traits with static
dispatch, and trait objects with dynamic dispatch. A call on a concrete type or
a type parameter resolves statically; a call through `*dyn Trait` or
`*mut dyn Trait` loads a slot from the receiver's compiler-built vtable. Neither
form allocates, and modifying methods borrow the receiver exclusively. User
methods declare their receiver explicitly as `self: S`, `self: *S`, or
`self: *mut S`; the compiler borrows a value receiver automatically when the
method expects a reference, and reborrows a `*mut S` receiver as `*S` for a
read-only method. A trait writes `Self` for the implementing type; a method may
declare a default body, and an impl that omits it inherits the body while any
supplied function overrides it. Default bodies dispatch symbolically to the
concrete implementation, including associated functions such as `Type::unit()`
and `Self::unit()`, so a static trait call remains an ordinary direct call.
Coercing `*S` or `*mut S` to a trait-object pointer preserves the reference kind
and requires an implemented, object-safe trait. Generic functions, structs, enums, inherent impls, associated functions,
traits, trait methods, and trait impls accept type parameters with inline or
`where` bounds; every reachable instance is monomorphized, so a generic call is
still one direct call with no dictionary or runtime type search. The type table
and the trait-object section of `docs/types.md` define the surface; owning
`Box<dyn Trait>`, upcasting between trait objects, and an `unsafe` layer are not
part of it yet. The trait-object examples in
`examples/032_trait_objects.sl` through
`examples/034_trait_objects_generic.sl` show dispatch end to end,
`examples/035_numeric_casts.sl` shows every cast family and its boundary, and
`examples/036_default_methods.sl` exercises required methods, defaults,
overrides, symbolic dispatch, supertraits, generic defaults, dynamic dispatch,
and associated default functions. `examples/037_iteration_protocols.sl` shows a
custom `IntoIterator` cursor and the direct `Iterator` fallback, and
`examples/038_index_protocols.sl` shows nested reads plus mutable indexing.

Global data is part of the surface: `const` and `static` items may appear at
file scope or in a block, `const func` may be called from a constant initializer,
and a static may hold aggregates, references, slices, strings, and trait
objects. Every initializer is evaluated at compile time, so a program has no
global constructors and no initialization order; a `static` is one allocation
with the layout and alignment of its type, and a `const` is substituted at every
use unless its address is taken. Only `static mut` is writable, and all of its
accesses are gated by `unsafe`, with a whole-program effect analysis keeping
calls honest. `examples/039_const_static.sl` shows constant values and constant
functions, `examples/040_global_data.sl` shows relocated aggregates, slices,
strings, and trait objects, and `examples/041_global_mutation.sl` shows
unsafe-gated mutable globals and block-scope statics.
