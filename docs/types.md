# Seal types

Status: accepted. This document is the reference for every type the compiler
knows, the properties later phases query, and the layout code generation relies
on. The semantic representation lives in `seal-ty`; sizes, alignments, and field
offsets are computed by `seal-layout`. The ownership rules behind these types are
decided in [`memory-model.md`](memory-model.md).

## Type list

Sizes and alignments are for a 64-bit target. `isize`, `usize`, and every
reference follow the target pointer width — 4 bytes on a 32-bit target — and
`*str` is always two words.

| Type | Spelling | Size (bytes) | Align | `Copy` | `Sized` | Destructor |
|------|----------|-------------:|------:|:------:|:-------:|:----------:|
| Boolean | `bool` | 1 | 1 | yes | yes | no |
| Character | `char` | 4 | 4 | yes | yes | no |
| Signed integer | `i8` | 1 | 1 | yes | yes | no |
| | `i16` | 2 | 2 | yes | yes | no |
| | `i32` | 4 | 4 | yes | yes | no |
| | `i64` | 8 | 8 | yes | yes | no |
| | `i128` | 16 | 16 | yes | yes | no |
| | `isize` | pointer | pointer | yes | yes | no |
| Unsigned integer | `u8` | 1 | 1 | yes | yes | no |
| | `u16` | 2 | 2 | yes | yes | no |
| | `u32` | 4 | 4 | yes | yes | no |
| | `u64` | 8 | 8 | yes | yes | no |
| | `u128` | 16 | 16 | yes | yes | no |
| | `usize` | pointer | pointer | yes | yes | no |
| Float | `f32` | 4 | 4 | yes | yes | no |
| | `f64` | 8 | 8 | yes | yes | no |
| Shared reference | `*T` | pointer | pointer | yes | yes | no |
| Exclusive reference | `*mut T` | pointer | pointer | yes | yes | no |
| String reference | `*str` | 2 × pointer | pointer | yes | yes | no |
| String pointee | `str` | unsized | unsized | no | no | no |
| Aggregate | `S` | computed | computed | no | yes, if every field is | no |
| Unit | not writable | 0 | 1 | yes | yes | no |
| Never | not writable | 0 | 1 | yes | yes | no |
| Error | internal | 0 | 1 | yes | yes | no |
| Inference variable | internal | unknown | unknown | no | no | no |

No current type runs a destructor: scalars, references, `str`, `()`, and `!` are
trivially disposable, and an aggregate inherits destructors from its fields once
owning heap types exist.

## Scalar types

### Signed integers

Six widths, all two's complement. A value of width `n` ranges from `−2^(n−1)` to
`2^(n−1) − 1`.

| Type | Bits | Range |
|------|-----:|-------|
| `i8` | 8 | `−128 ..= 127` |
| `i16` | 16 | `−32_768 ..= 32_767` |
| `i32` | 32 | `−2_147_483_648 ..= 2_147_483_647` |
| `i64` | 64 | `−9_223_372_036_854_775_808 ..= 9_223_372_036_854_775_807` |
| `i128` | 128 | `−2^127 ..= 2^127 − 1` |
| `isize` | target | `−2^(n−1) ..= 2^(n−1) − 1`, with `n` the pointer width |

### Unsigned integers

| Type | Bits | Range |
|------|-----:|-------|
| `u8` | 8 | `0 ..= 255` |
| `u16` | 16 | `0 ..= 65_535` |
| `u32` | 32 | `0 ..= 4_294_967_295` |
| `u64` | 64 | `0 ..= 18_446_744_073_709_551_615` |
| `u128` | 128 | `0 ..= 2^128 − 1` |
| `usize` | target | `0 ..= 2^n − 1`, with `n` the pointer width |

### Floating point

`f32` is IEEE 754 binary32 and `f64` is IEEE 754 binary64. Arithmetic and
comparisons follow IEEE semantics: ordered comparisons are false when either
operand is NaN, and `!=` is true when either operand is NaN.

### Quick reference

| Property | Behavior |
|----------|----------|
| Integer arithmetic | Wraps in two's complement; no overflow check |
| Division and remainder | Panic on a zero divisor, and on `MIN / -1` for signed integers; exit status 101 |
| Shift amounts | Masked to the operand width, so an oversized shift is defined |
| Implicit numeric conversions | None; operands must already have the expected type |
| Unsuffixed integer literal | Defaults to `i32` |
| Unsuffixed float literal | Defaults to `f64` |

## Literals

### Integers

- Decimal (`42`), binary (`0b1010`), octal (`0o755`), and hexadecimal (`0xFF`).
- `_` may separate digits (`1_000_000`, `0xdead_BEEF`). Separators are allowed
  between digits, including several in a row, but never at the start or end, next
  to the decimal point, next to the exponent marker, or next to the suffix.
- Every numeric type is a valid suffix: `0u8`, `-1i64`, `42usize`, `0xFFFFu16`.
- An integer literal with an `f32` or `f64` suffix is a floating-point literal.
- A negative literal is unary minus applied to a positive literal.

### Floating point

`1.5`, `1.`, `1e10`, `2.5e-3`, and `1.5f32` are valid. A float literal accepts
only the `f32` and `f64` suffixes; an integer suffix on a float is rejected.

### Boolean and character

`true` and `false` are the only `bool` literals. A `char` literal contains
exactly one Unicode scalar value, written directly (`'a'`, `'🦀'`) or as an
escape.

### Strings

A string literal is a `*str`: a pointer and a length into static read-only
memory. `str` itself is unsized, so it only appears behind a reference.

| Escape | Meaning |
|--------|---------|
| `\0` | NUL |
| `\t` | Tab |
| `\n` | Line feed |
| `\r` | Carriage return |
| `\"` | Double quote |
| `\'` | Single quote |
| `\\` | Backslash |
| `\xNN` | One byte in `0x00..=0x7F`, exactly two hexadecimal digits |
| `\u{…}` | One Unicode scalar value, one to six hexadecimal digits |

A raw line break is not allowed inside a string literal; it must be escaped.

## References

| Kind | Spelling | Read | Write | Overlap rule | Codegen attribute |
|------|----------|:----:|:-----:|--------------|-------------------|
| Shared | `*T` | yes | no | Any number of shared loans may overlap | `readonly` on pointer parameters (not `*str`) |
| Exclusive | `*mut T` | yes | yes | No other loan may overlap while live | `noalias` when the whole-program proof holds |

Both kinds are non-null, non-owning, and `Copy`; copying the pointer copies the
loan, so the pointee stays borrowed until the last use of every copy. Neither
kind supports arithmetic, casts, or null values. A reference is a machine
pointer, except `*str`, which is a two-word fat pointer.

Borrowing writes the place, not the type: `&place` creates `*T` and
`&mut place` creates `*mut T`. `&mut place` requires a writable place — a `mut`
binding or a `*mut T` dereference (`E3017`). Dereferencing is explicit (`*r`),
and field access through a reference is written `(*r).field`; there is no
automatic dereference.

Regions are inferred from liveness, so no lifetime syntax exists. A shared loan
conflicts with writes and moves (`E5001`, `E5002`); an exclusive loan conflicts
with every other loan and with reads, writes, and moves of the same storage
(`E5007`, `E5008`). Loans are tracked by place prefix, so `&mut p.x` leaves
`p.y` usable. A reborrow records the loans that authorized it: using the
reborrow does not conflict with them, while using the parent conflicts with the
reborrow.

A call is an access through every reference argument. Passing an exclusive
argument that overlaps a live loan is `E5009`, and a call whose reference
arguments may name the same storage when one is exclusive is `E5010`. The
`noalias` attribute is emitted only when the function body never reaches storage
through an unknown pointer and every call site proves the contract; `readonly`
on pointer parameters other than `*str` needs no such proof because the type
system already forbids writes through them.

## Structs

```seal
struct Point {
  x: i32,
  y: i32,
}

func origin() -> Point {
  Point { x: 0, y: 0 }
}
```

A struct is a **nominal** aggregate: its name defines a distinct type, field
order is part of the declaration, and field initializers may appear in any
order. A struct owns its fields, so it is never `Copy`; assignment, argument
passing, and return move it. Reading a `Copy` field copies only that field.
Fields initialize independently: `let mut p: Point; p.x = 1; p.y = 2;` is valid,
reading `p.x` before assigning it is an error, and a whole assignment initializes
every field at once. A field access on a reference requires an explicit
dereference and does not move out of the reference.

An aggregate may contain itself only behind a reference. A recursive field by
value has no finite size and is rejected during type checking before layout
runs.

## Unit, never, and internal types

`()` is the unit type: it is the result of a function with no `->` clause and of
expressions that produce no value. `!` is the never type, the type of expressions
that do not return, such as `return`, `break`, and `continue`. Neither type has a
source spelling in type position in v0.1; they appear in inferred signatures and
diagnostics. Both are zero-sized, aligned to 1, `Copy`, and `Sized`.

Two more kinds exist only inside the compiler:

- **`Error`** is produced during recovery so that one mistake does not cascade
  into unrelated diagnostics. It is zero-sized and never reaches codegen.
- **`Infer`** is an unresolved inference variable. Queries such as `Copy` and
  `Sized` answer `false` for it, and a caller that might move the value must
  resolve it first. No `Infer` type survives type checking.

## Type properties

Types are interned: two types are the same if and only if their `TyId`s are
equal, so equality is one integer comparison. `*T` and `*mut T` are distinct
kinds and never unify. The compiler answers three structural queries:

| Query | Rule |
|-------|------|
| `Copy` | Scalars, `()`, `!`, and every reference are copied bitwise. `str` and every struct are not, even when all their fields are `Copy`. |
| `Sized` | Every type has a compile-time size except `str` and an unresolved `Infer`. A struct is sized when every field is. |
| `needs_drop` | No current type runs a destructor. A struct will derive the answer from its fields once a type with a destructor exists. |

`Copy` and a destructor are mutually exclusive, and there is no user-defined
copy hook. A future opt-in `Copy` for structs derives the property from the
fields without changing the move path.

## Layout

Layout is deterministic and defined by the language, not by the host C ABI:

1. Scalars use natural alignment: alignment equals size. `bool` is 1 byte,
   `char` is 4 bytes, and every integer and float has its width.
2. Every reference uses the target pointer width. `*str` is two words: data and
   length, both pointer-sized.
3. Fields keep declaration order; there is no reordering, packing, or
   overlapping. Each field starts at the next multiple of its alignment.
4. A struct's alignment is the maximum of its fields' alignments, or 1 when it
   has no fields. Its size is the end of its last field rounded up to its own
   alignment, so trailing padding belongs to the value.
5. `()`, `!`, and `Error` are zero-sized with alignment 1. `str` has no layout of
   its own.

| Type | Size | Align | Offsets |
|------|-----:|------:|---------|
| `Point { x: i32, y: i32 }` | 8 | 4 | `x` at 0, `y` at 4 |
| `Mixed { flag: bool, value: i32, small: i8 }` | 12 | 4 | `flag` at 0, `value` at 4, `small` at 8 |
| `Segment { start: Point, end: Point }` | 16 | 4 | `start` at 0, `end` at 8 |
| `Empty {}` | 0 | 1 | none |
| `*i32` on a 64-bit target | 8 | 8 | none |
| `*str` on a 64-bit target | 16 | 8 | data at 0, length at 8 |

Code generation materializes one stack slot per local, uses struct GEPs for
fields, and passes references as pointers — or as a `{ data, length }` pair for
`*str`. `()` and `!` are zero-sized structs in the ABI.

## Growth

Every new type kind must define its layout, its `Copy`/`Sized`/`Drop` answers,
and its interning key before it can appear in a signature. Arrays and slices
will extend the layout rules with element stride and length; generics will
instantiate the same rules per concrete argument; owning heap types will add
field-derived destructors. None of these requires changing the identity,
layout, or ownership model above: `TyId` equality, declaration-order layout, and
the move/borrow rules stay the same.
