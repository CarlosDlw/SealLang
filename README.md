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
| Diagnostics | Stable error codes with long-form explanations (`--explain`) |

## Quick start

```text
cargo build --release
./target/release/sealc -g examples/006_structs.sl -o /tmp/seal-structs
/tmp/seal-structs; echo $?
```

```text
10
```

`sealc` runs the whole pipeline: it type-checks, borrow-checks, lowers checked
MIR to LLVM IR, optimizes, and links an executable. `--emit=check` stops after
analysis, while `--emit=obj` and `--emit=asm` write intermediate artifacts.

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

`match` is an expression over enum variants, structs, arrays, and scalar values.
A braced block is an ordinary expression, so an arm body can run statements and
produce a value. Patterns nest: they match unit, positional, and named variants,
destructure structs, match every element of a fixed-size array, bind fields,
compare integer, float, character, and boolean literals, range over integers and
characters (`0..=9`, `'a'..='z'`), bind a whole value while narrowing it
(`name @ 0..=9`), and combine alternatives with `|`. An arm may carry an `if`
guard, which sees only `Copy` bindings because a failed guard must leave the
scrutinee available for the next arm. Coverage is proved by the usefulness
algorithm: enums and `bool` are exhaustive once every constructor is covered,
integer and character ranges partition their domain so adjacent ranges can be
exhaustive, floats need a catch-all, and an arm that no value can reach is
`E3033`.

## Command line

```text
sealc [OPTIONS] <INPUT>...
sealc --emit-runtime <PATH>
sealc --explain <CODE>
```

| Option | Effect |
|--------|--------|
| `--emit=check` | Stop after borrow checking; write nothing |
| `--emit=obj` | Write a relocatable object |
| `--emit=asm` | Write target assembly |
| `--emit=exe` | Link a native executable (default) |
| `-O0` … `-O3`, `-O` | Select the optimization level (`-O` means 3) |
| `-g` | Embed DWARF debug information |
| `-o <PATH>` | Write the artifact to `PATH` (one input only) |
| `--emit-runtime <PATH>` | Export the embedded runtime archive |
| `--runtime-archive <PATH>` | Link a different runtime archive |
| `--explain <CODE>` | Print the long-form explanation of a diagnostic |

Without `-o`, `file.sl` produces `file`, `file.o`, or `file.s` next to the
input. The exit status is `0` on success, `1` after a compile error, `2` after a
usage error, and `101` after an internal compiler error or a runtime panic.

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
| Arrays | `[T; N]` | Fixed length, owns its elements, and is `Copy` when `T` is |
| Slices | `[T]`, `*[T]`, `*mut [T]` | Borrowed views over contiguous elements; references carry data and length |
| Aggregates | `struct S { … }` | Nominal, owns its fields, and is not `Copy` |
| Enums | `enum E { … }` | Nominal sum type with unit, positional, or named variants; owns the payload and is not `Copy` |
| Methods | `impl S { … }` | Inherent methods and associated functions; static calls, no vtable |
| Traits | `trait T { … }`, `impl T for S { … }` | Static dispatch: each call binds to the concrete implementation of the receiver's type |

There are no implicit numeric conversions: every arithmetic and comparison
operand must already have the expected type, and an unsuffixed integer literal
defaults to `i32` while an unsuffixed float defaults to `f64`.

See [`docs/types.md`](docs/types.md) for sizes, alignments, ranges, literal
syntax, `Copy`/`Sized`/`Drop`, reference rules, and the layout algorithm.

## Documentation

| Document | Contents |
|----------|----------|
| [`docs/memory-model.md`](docs/memory-model.md) | Ownership, moves, borrowing, allocation, and the decisions behind them |
| [`docs/types.md`](docs/types.md) | The complete type list, properties, literals, layout, and ABI representation |
| [`docs/architecture.md`](docs/architecture.md) | Compiler phases, crate responsibilities, and public invariants |
| [`docs/codegen.md`](docs/codegen.md) | LLVM lowering, ABI, debug information, runtime, and optimization |

## Status

The current language surface is functions, nominal structs and enums,
fixed-size arrays, slices, shared and exclusive references, `let` bindings with
definite initialization, `if`, `while`, `break`, `continue`, `return`, `match`
expressions with structural patterns, guards, and exhaustiveness checking, the
usual arithmetic, comparison, logical, and bitwise operators, compiler-owned intrinsic
methods on integers, floats, `bool`, `char`, arrays, and slices, inherent `impl`
methods and associated functions on nominal types, and traits with static
dispatch. Every call resolves statically: there is no vtable, dynamic dispatch,
or hidden allocation, and modifying methods borrow the receiver exclusively.
User methods declare their receiver explicitly as `self: S`, `self: *S`, or
`self: *mut S`; the compiler borrows a value receiver automatically when the
method expects a reference, and reborrows a `*mut S` receiver as `*S` for a
read-only method. A trait writes `Self` for the implementing type and every
`impl Trait for S` binds one concrete function per method, so a trait call is an
ordinary direct call. Generics, trait objects, and an `unsafe` layer are not part
of the surface yet; the memory model records how they fit without redesigning the
type system.
