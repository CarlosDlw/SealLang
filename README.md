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
whole. `*T` grants shared access and `*mut T` grants exclusive access, and the
two never unify. Exclusive arguments are checked at every call site (`E5009`,
`E5010`), which is what allows the backend to emit `noalias` without turning
safe code into undefined behavior.

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

## Types

| Family | Types | Notes |
|--------|-------|-------|
| Signed integers | `i8` … `i128`, `isize` | Two's complement; `isize` follows the target pointer width |
| Unsigned integers | `u8` … `u128`, `usize` | Wrapping arithmetic; `usize` follows the target pointer width |
| Floating point | `f32`, `f64` | IEEE 754 binary32 and binary64 |
| Text | `bool`, `char`, `str`, `*str` | `char` is a Unicode scalar value; `str` is unsized and only usable behind a reference |
| References | `*T`, `*mut T` | Non-null, non-owning, and `Copy`; shared or exclusive |
| Aggregates | `struct S { … }` | Nominal, owns its fields, and is not `Copy` |

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

The current language surface is functions, nominal structs, shared and exclusive
references, `let` bindings with definite initialization, `if`, `while`, `break`,
`continue`, `return`, and the usual arithmetic, comparison, logical, and bitwise
operators. Arrays, slices, generics, methods, enums, and an `unsafe` layer are
not part of the surface yet; the memory model records how they fit without
redesigning the type system.
