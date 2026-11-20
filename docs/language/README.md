# Seal language guide

This directory teaches the language from the user's point of view. The
documents in the [parent directory](../) are the normative design and compiler
references; the chapters here explain how to use the implemented surface
correctly, with complete rules, worked examples, and the contracts behind
unsafe operations.

## Chapters

| Chapter | Covers |
|---------|--------|
| [`copy.md`](copy.md) | Opt-in `Copy` for structs and enums, generic bounds, fields and variants, `Drop`, references, patterns, loops, statics, and diagnostics |
| [`raw-pointers.md`](raw-pointers.md) | `*raw T` and `*raw mut T`, casts, arithmetic, fat pointers, `unsafe`, constants, provenance, diagnostics, and patterns |
| [`ffi.md`](ffi.md) | Complete guide to C interop: `extern "c"` functions, statics, structs, opaque types, variadics and `c"..."`, function pointers and callbacks, buffers, linking, the safety contract, constant evaluation, diagnostics, and the acceptance matrix |
| [`first-class-functions.md`](first-class-functions.md) | `func` and `unsafe func` types, closures and capture, `move`, `Fn`/`FnMut`/`FnOnce` bounds, coercion, drop and borrow rules, ABI, diagnostics, and limits |

## Reading conventions

- Code blocks are fragments that show the smallest context needed for the
  rule. The complete programs are the files under [`examples/`](../../examples/)
  and the end-to-end example at the close of each chapter.
- `T`, `U`, and `V` stand for concrete types. `T` must be `Sized` whenever the
  operation stores or advances by whole values.
- `Same` in a method table means the receiver's pointer type, including its
  mutability and pointee.
- Error codes are stable. Run `sealc explain <CODE>` for the long diagnostic.
- The reference documents remain authoritative when a compiler invariant and a
  guide sentence could be read differently:
  [`../types.md`](../types.md), [`../memory-model.md`](../memory-model.md),
  [`../architecture.md`](../architecture.md), and [`../codegen.md`](../codegen.md).

## Example programs

Every chapter points at a runnable file under [`examples/`](../../examples/).
The acceptance suite compiles each example at `-O0` and `-O2`, runs it, and
asserts the documented exit status.
