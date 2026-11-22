# Seal types

Status: accepted. This document is the reference for every type the compiler
knows, the properties later phases query, and the layout code generation relies
on. The semantic representation lives in `seal-ty`; sizes, alignments, and field
offsets are computed by `seal-layout`. The ownership rules behind these types are
decided in [`memory-model.md`](memory-model.md).

## Type list

Sizes and alignments are for a 64-bit target. `isize`, `usize`, and every
reference follow the target pointer width — 4 bytes on a 32-bit target — while
`*str` and a trait-object pointer are always two words.

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
| Slice reference | `*[T]`, `*mut [T]` | 2 × pointer | pointer | yes | yes | no |
| Slice pointee | `[T]` | unsized | unsized | no | no | no |
| Trait-object reference | `*dyn Trait`, `*mut dyn Trait` | 2 × pointer | pointer | yes | yes | no |
| Trait-object pointee | `dyn Trait` | unsized | unsized | no | no | no |
| Raw pointer | `*raw T`, `*raw mut T` | pointer | pointer | yes | yes | no |
| Raw string pointer | `*raw str`, `*raw mut str` | 2 × pointer | pointer | yes | yes | no |
| Raw slice pointer | `*raw [T]`, `*raw mut [T]` | 2 × pointer | pointer | yes | yes | no |
| Raw trait-object pointer | `*raw dyn Trait`, `*raw mut dyn Trait` | 2 × pointer | pointer | yes | yes | no |
| C function signature | not written alone | unsized | unsized | no | no | no |
| C function pointer | `*raw extern "c" func(...) -> R` | pointer | pointer | yes | yes | no |
| Seal function pointer | `func(...) -> R`, `unsafe func(...) -> R` | pointer | pointer | yes | yes | no |
| Closure | inferred, not nameable | computed | computed | structural | yes | no |
| Aggregate | `S` | computed | computed | no | yes, if every field is | no |
| C-layout aggregate | `extern struct S` | computed | computed | opt-in | yes, if every field is | opt-in |
| Opaque type | `extern type T`, `c_void` | unsized | unsized | no | no | no |
| Enum | `E` | computed | computed | no | yes, if every variant field is | no |
| Tuple | `(T, U)`, `(T,)` | computed | computed | yes, if every element is | yes, if every element is | no |
| Array | `[T; N]` | N × size(T) | align(T) | yes, if `T` is | yes, if `T` is | no |
| Unit | `()` | 0 | 1 | yes | yes | no |
| Never | not writable | 0 | 1 | yes | yes | no |
| Error | internal | 0 | 1 | yes | yes | no |
| Inference variable | internal | unknown | unknown | no | no | no |
| Type parameter | `T` | after instantiation | after instantiation | by bound | yes | no |

The type introspection built-ins `type_name`, `type_of`, `size_of`, and
`align_of` report these names, sizes, and alignments; their rules are in
[`language/builtins.md`](language/builtins.md).

The C scalar names described under [C scalar types](#c-scalar-types) are
transparent aliases of the integer and float types above. Their width and
signedness come from the target's C ABI, so `c_int` is `i32` everywhere and
`c_long` is `i64` on LP64 targets and `i32` on 64-bit Windows.

A type runs a destructor when it declares `impl Drop`; otherwise it inherits
destructors from its fields or elements. Scalars, references, `str`, `()`, and
`!` are trivially disposable, so dropping them runs nothing. A `static` is never
dropped, and constant evaluation rejects a drop that would run a destructor
(`E4028`).

## Generic types

A generic declaration introduces type parameters and is checked once, against
those parameters treated as rigid types. Every use then instantiates it with
concrete arguments, and code generation emits one instance per reachable
`(definition, type arguments)` pair.

### Declarations and bounds

```seal
struct Pair<T, U> {
  first: T,
  second: U,
}

func read<T, U>(value: *T) -> U where T: Convert<U> {
  value.convert()
}

trait Convert<T> {
  func convert(self: *Self) -> T;
}

impl<T: Copy> Convert<T> for Labeled<T> {
  func convert(self: *Labeled<T>) -> T {
    (*self).label
  }
}
```

- Parameters are declared as `<T, U: Bound>` after an item, impl, or method
  name. A `where` clause after the header declares the same bounds.
- Bounds are `Copy` (structural, checked against the concrete type) or a trait
  reference such as `Convert<U>` (closed-world: the program must contain one
  matching impl).
- `Self` is the implicit first parameter of a trait and the concrete self type
  in an impl; it is rejected in a free function.
- A method inside an impl or trait inherits its parent's parameters; it may
  declare its own after them.
- Type parameters are `Sized` by default. There is no `?Sized`, no default type
  argument, no specialization, no associated type, and no const generic in this
  version.

### Inference and explicit arguments

- A type annotation must spell every argument: `Pair<i32, bool>`. A bare `Pair`
  is an error.
- An expression may omit arguments; they are inferred from argument types,
  struct field initializers, an enum variant payload, the receiver type, and the
  expected return or binding type. Inference runs both directions, so
  `let x: i32 = make();` instantiates `make<T>` with `i32`.
- Turbofish makes an argument explicit: `identity::<i32>(value)`,
  `Pair::<i32, bool> { … }`, `Option::<i32>::None`, `value.method::<u8>(…)`.
- If no source determines a parameter, the program is rejected naming it; it is
  never defaulted or erased.

### Layout and cost

Each instantiation is a distinct concrete type. `Pair<i32, bool>` and
`Pair<u8, i32>` have their own field types, sizes, alignments, and offsets;
`Option<Pair<i32, bool>>` gets a payload sized for that pair. Recursion must go
through a reference (`*Node<T>` or an enum holding one), so every layout stays
finite. Type arguments have no runtime representation: they cost nothing at run
time and are not stored in the value.

The costs are compile time and binary size. Every reachable instance is emitted
once, a generic call resolves to a direct call, and no allocation, boxing, or
dictionary is introduced. A trait object is the separate opt-in dynamic path:
its vtable is built at compile time for the concrete instance and adds no
runtime allocation. Monomorphization is bounded: a recursive instantiation chain
deeper than 64 (`E7001`) or more than 4096 distinct instances (`E7004`) is rejected
with the instance chain. As a reference measurement, the five generic examples
link to about 16 KiB each at `-O2` on 64-bit Linux, and a stress program with
100 distinct instances stays under 1 MiB; the test suite enforces that bound.

### Compatibility with non-generic code

Generics are additive. A program without type parameters keeps its symbols
(`seal.<name>` and `seal.<Type>.<name>`), ABI, layouts, diagnostics, and
output; no instance suffix is added and no extra MIR body or LLVM function is
emitted. A generic definition that is never instantiated is not emitted at all.
Existing non-generic code needs no migration; adding a type parameter changes
only that item's symbol, which is internal to one compilation.

## Function types and closures

A Seal function pointer is written `func(A, B) -> R`; the result may be omitted
for `()`. It is a sized, `Copy`, one-word value with no destructor, no null
value, no equality, and no casts to or from integers or raw pointers. `unsafe`
is part of the type, and calling an `unsafe func` value requires an `unsafe`
block. These are the internal Seal ABI, never the C ABI; a C signature is a
separate `FnSig` type behind `*raw`.

`FnPtrDef` is interned by its ordered parameters, result, and safety marker, so
two spellings of the same signature share one `TyId`. Function items coerce to
the matching pointer type. A capture-free closure coerces too, through an
internal trampoline; a closure with captures is rejected (`E3098`). Safe
functions convert to safe or unsafe pointers, but unsafe pointers never convert
back to safe ones.

A closure is represented as a synthetic nominal aggregate whose fields are its
captures, plus a lifted body. The type is anonymous and cannot be written, but
it behaves like any other `Adt`: it is `Sized`, it is `Copy` exactly when every
field is, and it needs no destructor of its own because the environment drops
its fields. A shared capture stores `*T`; an exclusive capture stores `*mut T`
and makes the closure non-`Copy`; a `move` capture stores the value. The kind
(`Fn`, `FnMut`, or `FnOnce`) is a property of the closure, not of its layout.

The trait bounds `Fn(A, B) -> R`, `FnMut(A)`, and `FnOnce() -> R` are callable
contracts, not storable trait objects. A value satisfying them is either a
closure or a safe function pointer; `dyn Fn` and `impl Fn` are not part of the
type surface. See
[`language/first-class-functions.md`](language/first-class-functions.md) for the
full rules.

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

### C scalar types

The C scalar names resolve to the integer or float type with the width and
signedness the target's C ABI uses. They are transparent aliases: `c_int` is
`i32` on every supported target, and a value of one type is accepted wherever
the other is expected.

| Name | C type | Seal type |
|------|--------|-----------|
| `c_char` | `char` | `i8` or `u8`, according to the target |
| `c_schar` | `signed char` | `i8` |
| `c_uchar` | `unsigned char` | `u8` |
| `c_short` | `short` | `i16` |
| `c_ushort` | `unsigned short` | `u16` |
| `c_int` | `int` | `i32` |
| `c_uint` | `unsigned int` | `u32` |
| `c_long` | `long` | `i64` on LP64, `i32` on LLP64 and 32-bit targets |
| `c_ulong` | `unsigned long` | `u64` on LP64, `u32` on LLP64 and 32-bit targets |
| `c_longlong` | `long long` | `i64` |
| `c_ulonglong` | `unsigned long long` | `u64` |
| `c_float` | `float` | `f32` |
| `c_double` | `double` | `f64` |
| `c_bool` | `_Bool` inside an aggregate | `u8`, holding 0 or 1 |
| `c_wchar` | `wchar_t` | `u16` on Windows; `i32` elsewhere |
| `size_t` | `size_t` | `usize` |
| `ssize_t` | `ssize_t` | `isize` |
| `ptrdiff_t` | `ptrdiff_t` | `isize` |
| `intptr_t` | `intptr_t` | `isize` |
| `uintptr_t` | `uintptr_t` | `usize` |

Plain `char` is signed on Windows, Darwin, and the x86 family, and unsigned on
the ARM, AArch64, RISC-V, and WebAssembly ABIs. `wchar_t` is a 16-bit unsigned
type on Windows and a 32-bit signed type elsewhere. The opaque `c_void` type is
not a scalar; it belongs to the C interop layer.

These names are type names, not literal suffixes: write `1 as c_int` or annotate
the binding, never `1c_int`. Declaring C functions and statics that use these
types is covered in [`language/ffi.md`](language/ffi.md).

## C interop types and layout

`extern struct` produces an ordinary aggregate with `AdtRepr::C`. Its fields
keep declaration order, each starts at the C alignment of its type, and the
whole struct is padded to its maximum alignment. Those are the same offsets the
host C compiler computes, so the language layout and the C layout are checked by
the fixture instead of being assumed. A nested `extern struct` and a fixed-size
array of C-safe elements are laid out recursively.

The field types form the following matrix:

| C declaration | Seal spelling | LLVM representation |
|---------------|---------------|---------------------|
| `struct S { ... }` | `extern struct S { ... }` | named struct in declaration order |
| `union U` | none | rejected: no union layout |
| `enum E` | `c_int`/`c_uint` plus constants | a C integer |
| incomplete `struct T` | `extern type T` | no value type; only `ptr` |
| `void` pointee | `c_void` | no value type; only `ptr` |
| `_Bool` | `c_bool` (field) / `bool` (direct parameter or result) | `i8` / `i1` |
| `char` | `c_char` | `i8` or `u8` per target |
| `size_t` | `size_t` | pointer-sized integer |

An `extern type` and `c_void` are unsized: they have no fields, no layout, no
`Copy`, no destructor, and no by-value use. They appear only behind `*raw` and
`*raw mut`; the pointer is one word and is FFI-safe. A by-value use or a
dereference is `E3087`.

At a C boundary, a small `extern struct` is not passed with its raw LLVM struct
type. Code generation classifies it with the target ABI and coerces it to
integer or floating-point pieces; a struct larger than the target's register
limit is passed with `byval` and returned through `sret` on System V, or passed
and returned indirectly on Microsoft x64. A coerced integer keeps its exact
width: a 3-byte struct becomes `i24`, never a padded `i32`, so the register
contains only the bytes C placed there. The complete struct remains a normal
Seal aggregate inside the program, so field access, construction, pattern
matching, and drop glue do not depend on the boundary representation.

A Seal struct without `extern` keeps `AdtRepr::Rust`, is still rejected at the
boundary with `E3084`, and the diagnostic suggests declaring it with
`extern struct`. A struct field that is `bool`, zero-sized, or not FFI-safe is
rejected in the declaration (`E3085`/`E3084`), so an invalid C layout can never
reach code generation.

### Explicit numeric casts

`value as Type` converts a value between primitive numeric types. The operator
binds tighter than the multiplicative operators and looser than prefix
operators, and chains associate left to right:

```seal
let truncated = 300 as u8;        // 44: narrowing keeps the low bits
let extended = -5 as i64;         // -5: signed widening sign-extends
let scaled = 7 as f64;            // 7.0: integer to float rounds
let rounded = 3.9 as i32;         // 3: float to integer truncates toward zero
let chained = 200 as i8 as i64;   // -56
```

Only these pairs are accepted:

| Source | Target | Result |
|--------|--------|--------|
| any integer | any integer | Same width preserves the bit pattern; narrowing truncates; widening sign-extends a signed source and zero-extends an unsigned one |
| any integer | `f32`/`f64` | The mathematical value rounded to nearest, ties to even; a value beyond the destination range becomes infinity of the same sign |
| `f32`/`f64` | any integer | Truncates toward zero; NaN becomes `0`; values above the maximum saturate to the maximum; values below the minimum saturate to the minimum |
| `f32` | `f64` | Exact widening, including signed zero and NaN |
| `f64` | `f32` | Rounded to nearest, ties to even; overflow becomes infinity of the same sign |
| numeric `T` | the same `T` | A no-op |

A cast is infallible and pure: it never panics, allocates, moves a non-`Copy`
value, creates a loan, or changes the operand's initialization state. The
operand is inferred without the target as an expectation, so `1 as u8` first
types `1` as `i32` and then truncates; the literal is not silently a `u8` that
must fit. A cast that loses information is legal — that is why it must be
written explicitly.

Reference, raw-pointer, and pointer-sized integer casts are a separate family
described under [Raw pointers](#raw-pointers). They are not numeric conversions:
going through `usize` and back is an explicit round trip, and a raw pointer's
address is not available in a constant (`E4030`).

`bool`, `char`, enums, slices, arrays, tuples, structs, trait objects, `str`,
unit, and the never type have no numeric cast (`E3048`); a reference converts
only to a raw pointer. Use
`bool.to_i32()` and `char.to_u32()` for those two named conversions. A cast
whose source or target is a type parameter is rejected (`E3049`), because there
is no numeric-bound trait from which to select a concrete conversion.

### Quick reference

| Property | Behavior |
|----------|----------|
| Integer arithmetic | Wraps in two's complement; no overflow check |
| Division and remainder | Panic on a zero divisor, and on `MIN / -1` for signed integers; exit status 101 |
| Shift amounts | Masked to the operand width, so an oversized shift is defined |
| Implicit numeric conversions | None; operands must already have the expected type |
| Explicit numeric conversion | `value as Type`, numeric pairs only; may lose information |
| Unsuffixed integer literal | Defaults to `i32` |
| Unsuffixed float literal | Defaults to `f64` |

## Literals

### Integers

- Decimal (`42`), binary (`0b1010`), octal (`0o755`), and hexadecimal (`0xFF`).
- `_` may separate digits (`1_000_000`, `0xdead_BEEF`). Separators are allowed
  between digits, including several in a row, but never at the start or end, next
  to the decimal point, next to the exponent marker, or next to the suffix.
- Every built-in numeric type is a valid suffix: `0u8`, `-1i64`, `42usize`,
  `0xFFFFu16`. The C scalar names are not suffixes.
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

### C strings

`c"..."` is a C string literal. Its type is `*raw u8`: a thin, read-only raw
pointer to static bytes followed by a NUL terminator. It carries no length and
is not a `*str`, so it cannot call `str` methods or be used where `*str` is
expected. It is the direct spelling for a C `const char *` parameter.

```seal
let text: *raw u8 = c"value=%d";
let empty: *raw u8 = c"";
```

The same escape table applies. A NUL written inside the literal (`c"a\0b"`)
is `E3090`, because C would stop at the first NUL and the rest of the literal
would be unreachable; end the string instead. An empty C string is valid and
points at one NUL byte.

### String methods

Methods on `str` are called through a `*str` and never allocate. Every index is
a **byte offset**, never a scalar count. A valid index is in `0..=len`; an index
at `len` names the end, while any index past it is invalid. A slice is
end-exclusive, so `slice(start, end)` covers `end - start` bytes.

| Method | Signature | Behavior |
|--------|-----------|----------|
| `len` | `(self) -> usize` | Byte length |
| `is_empty` | `(self) -> bool` | Whether the byte length is zero |
| `is_char_boundary` | `(self, index: usize) -> bool` | Whether `index` is a scalar boundary; true at `0` and `len` |
| `char_at` | `(self, index: usize) -> char` | Decodes the scalar starting at a boundary; panics otherwise |
| `slice` | `(self, start: usize, end: usize) -> *str` | Checked borrowed substring; panics on an invalid range |
| `get` | `(self, start: usize, end: usize) -> Option<*str>` | Total counterpart of `slice`; returns `Option::None` instead of panicking |
| `split_at` | `(self, index: usize) -> (*str, *str)` | Two borrowed halves at a boundary; panics otherwise |
| `as_bytes` | `(self) -> *[u8]` | Borrowed byte view with the same loan |
| `bytes` | `(self) -> Bytes` | Iterator over the exact UTF-8 bytes |
| `chars` | `(self) -> Chars` | Iterator over Unicode scalar values |
| `char_indices` | `(self) -> CharIndices` | Iterator over `(byte offset, char)` pairs |
| `starts_with` | `(self, prefix: *str) -> bool` | Prefix test; the empty prefix matches |
| `ends_with` | `(self, suffix: *str) -> bool` | Suffix test; the empty suffix matches |
| `contains` | `(self, needle: *str) -> bool` | Whether `find` succeeds |
| `find` | `(self, needle: *str) -> Option<usize>` | First byte index, or `Option::None` |
| `split` | `(self, separator: *str) -> Split` | Iterator over the pieces between separators |
| `split_once` | `(self, separator: *str) -> Option<(*str, *str)>` | The first split only |
| `lines` | `(self) -> Lines` | Iterator over `\n`-separated lines |
| `split_whitespace` | `(self) -> SplitWhitespace` | Iterator over Unicode-whitespace-separated words |
| `trim` | `(self) -> *str` | Both ends trimmed of Unicode whitespace |
| `trim_start` | `(self) -> *str` | Leading Unicode whitespace trimmed |
| `trim_end` | `(self) -> *str` | Trailing Unicode whitespace trimmed |
| `eq_ignore_ascii_case` | `(self, other: *str) -> bool` | ASCII-only case folding; non-ASCII bytes compare exactly |

`slice` panics only for a range that is reversed, ends past `len`, or starts or
ends inside a multibyte scalar. The messages are `slice range starts after it
ends`, `slice range end is out of bounds`, `slice range starts inside a
character`, and `slice range ends inside a character`. `get` performs the same
checks and accepts every `usize` pair without panicking. `char_at` panics with
`char_at index is out of bounds` at or past `len` and with `char_at index is
not a char boundary` inside a scalar. `split_at` panics with `split_at index is
out of bounds` or `split_at index is not a char boundary`.

Every view returned by `slice`, `get`, `split_at`, `split`, `split_once`,
`lines`, `split_whitespace`, or `trim` borrows the source storage and cannot
outlive it. Iterator `next` is called only while `has_next` is true; `for`
maintains that contract. `Split` and `Lines` panic with `split iterator is
exhausted` and `lines iterator is exhausted` if `next` is called directly after
exhaustion. Search uses Two-Way, so `find` and everything built on it is
`O(n + m)` in the worst case with constant preprocessing space. `find("")` is
`Option::Some(0)`. A non-empty separator preserves leading, trailing, and
consecutive empty pieces; the empty separator visits every scalar with an empty
piece at each end, so splitting an empty string on `""` yields two empty pieces.
`lines` removes a `\r` immediately before `\n` and does not produce a final
empty line for a trailing newline. `split_whitespace` uses the Unicode
`White_Space` property and never yields empty items.

`from_utf8(*[u8]) -> Option<*str>` is the safe constructor: it validates the
complete encoding, rejects lone continuation bytes, truncated sequences,
overlong encodings, surrogates, and values above `U+10FFFF`, and returns a view
whose loan keeps the bytes alive. The unsafe function
`from_utf8_unchecked(*[u8]) -> *str` skips validation and is valid only when the
caller has already established UTF-8; it still returns a borrowing view. `*[u8].as_str_unchecked()` is the
underlying unsafe intrinsic. The raw-pointer operation `*raw [u8].as_str()` is
different: it produces a `*raw str` with no checked loan, and it also trusts the
caller's UTF-8 promise. Safe code should use `from_utf8`.

No method on `str` allocates, formats, or copies the text. An owning `String`
can be added later without changing `str`'s layout or safety contract: it will
need an owning buffer, an allocator API, and a loan-preserving `String::as_str`.
`str` exposes no `Deref` coercion, so `String` will use an explicit `.as_str()`.

## References

| Kind | Spelling | Read | Write | Overlap rule | Codegen attribute |
|------|----------|:----:|:-----:|--------------|-------------------|
| Shared | `*T` | yes | no | Any number of shared loans may overlap | `readonly` on pointer parameters (not `*str` or `*dyn Trait`) |
| Exclusive | `*mut T` | yes | yes | No other loan may overlap while live | `noalias` when the whole-program proof holds |

Both kinds are non-null, non-owning, and `Copy`; copying the pointer copies the
loan, so the pointee stays borrowed until the last use of every copy. Neither
kind supports arithmetic, pointer casts, or null values. A reference is a machine
pointer, except `*str`, `*[T]`, and `*mut [T]`, which are two-word fat pointers
holding a data address and a length, and `*dyn Trait`/`*mut dyn Trait`, whose
second word addresses the vtable.

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
through an unknown pointer or a dynamic call and every call site proves the
contract; `readonly` on pointer parameters other than `*str` or `*dyn Trait`
needs no such proof because the type system already forbids writes through them.
A trait-object pointer carries the same loan as its concrete source, so passing
one is an access through every reference argument like any other.

### Raw pointers

Raw pointers are the opt-in unsafe pointer type. They are written `*raw T` and
`*raw mut T`: `raw` is contextual after `*`, so it remains an ordinary
identifier elsewhere. The type is nullable and supports arithmetic and casts;
it is not a reference and never unifies with `*T` or `*mut T`. A raw pointer is
`Copy`, `Sized`, has no destructor, and carries no borrow-checked loan. Its
layout is one word for a sized pointee and two words for `str`, a slice, or a
trait object; the second word is the length or vtable metadata.

```seal
static VALUE: i32 = 7;

const READ_ONLY: *raw i32 = &VALUE as *raw i32;
const NULL: *raw i32 = 0usize as *raw i32;

func writable(value: *mut i32) -> *raw mut i32 {
  value as *raw mut i32
}
```

A `const` may hold a read-only pointer into a static; a writable pointer is
formed at runtime, because constant evaluation cannot contain a mutable
reference (`E4012`) or write a static (`E4015`).

A reference can become a raw pointer with the same pointee and no more
permission: `&place as *raw T`, `&mut place as *raw mut T`, and the downgrade
`&mut place as *raw T` are accepted. A shared reference cannot become a mutable
raw pointer (`E3074`). Raw-to-raw casts may change the pointee and may discard
or acquire mutability; they may not turn a thin pointer into a fat pointer or
the reverse. Only `usize` and `isize` convert to and from a thin raw pointer;
other integer widths and floats are `E3074`. A raw pointer to `str`, a slice,
or a trait object is a fat value and cannot be forged from an integer.

Raw pointer equality compares the data address, not the value it points to and
not the metadata word. `==` and `!=` are available without `unsafe`; ordering
operators are not. `is_null()`, `addr()`, `expose_addr()`, and `with_addr()` are
safe queries. `len()` is available on raw slice and string pointers, and
`data()` discards metadata, producing a thin pointer with the same mutability
(element type for a slice, `u8` for a string, and `()` for a trait object).
Pointer arithmetic (`add`, `sub`, `offset`, `byte_add`, `byte_offset`,
`wrapping_add`, `wrapping_sub`, `wrapping_offset`, and `offset_from`) requires a
sized pointee; call `data()` first for a fat pointer.

A C function pointer is a thin raw pointer whose pointee is a C function
signature: `*raw extern "c" func(i32) -> c_int`. The signature is unsized and
never appears by value. The pointer is one word, `Copy`, and FFI-safe. It is
always read-only (`*raw mut extern func` is `E3089`), and every fixed parameter
and result must be FFI-safe. A Seal function converts to it with an explicit
cast whose instantiated signature matches exactly; the compiler emits a C-ABI
trampoline when the internal ABI differs. Taking the address, forging the
pointer from an integer, and calling through it require `unsafe`. See
[External functions and statics](language/ffi.md#c-function-pointer-types).

Dereferencing, reading, writing, copying, and non-wrapping arithmetic are unsafe
operations and require an `unsafe { ... }` block. `*raw T` is read-only for
writes and `*raw mut T` permits them (`E3075`). `read`, `read_unaligned`,
`write`, `write_unaligned`, `copy`, and `copy_nonoverlapping` lower to the
corresponding memory operations; `copy` has `memmove` semantics and
`copy_nonoverlapping` has `memcpy` semantics. Writing through a raw pointer does
not run a destructor for the old value.

The full language guide is [`docs/language/raw-pointers.md`](language/raw-pointers.md).

## Constants, statics, and global data

A `const` is a compile-time value with no storage of its own: each use is
replaced by the value. A `static` is one allocation with program lifetime,
created before the program starts and addressable through a reference. Both must
have an explicit type and an initializer, and both are evaluated at compile time.

```seal
const LIMIT: usize = 1 << 20;

static GREETING: *str = "hello";
static TABLE: [i32; 4] = [1, 2, 3, 4];
static mut COUNTER: i32 = 0;

const func twice(value: i32) -> i32 {
    value * 2
}
```

The type of a `const` or `static` must be `Sized` (`E3056`, `E3057`); `str`,
`[T]`, and bare trait objects are only usable behind a reference. Neither item
may declare generic parameters or mention one from the enclosing scope
(`E3060`): there is exactly one value per declaration.

`const` and `static` live in the value namespace together with functions,
parameters, and bindings. A const/static name may collide with a type name but
not with another value name (`E2068`). Items are order-independent: a `const`
may refer to a `static` declared later, subject to cycle detection.

Reading a `static` by value copies it, so the type must be `Copy`; a struct or
enum with an opt-in implementation follows the same rule. `drop(STATIC)` is a
no-op for a `Copy` static and `E3058` for a non-`Copy` one. A non-`Copy` static
can be borrowed or read field by field but not moved. A
`const`, by contrast, produces a fresh value at every use, so reading a
non-`Copy` const by value is allowed; `&CONST` borrows one compiler-owned
constant allocation. The checker records each path as a place rooted at a
binding, a const, or a static, and only a binding root participates in
definite-initialization tracking: a const or static is always initialized.
Every reference to the same static denotes the same allocation, and a static is
never dropped. Immutable statics are read-only after initialization (`E3059`); mutable
storage is written `static mut` and is only accessible inside an `unsafe { ... }`
block (`E3063`).

```seal
func increment() {
    unsafe {
        COUNTER = COUNTER + 1;
    }
}
```

`unsafe { ... }` is an expression whose value is the block's value. In this
feature it unlocks `static mut` places only; it does not relax type checking,
borrow checking, initialization checking, or the global effect analysis, and it
has no runtime representation. The checker tracks the number of enclosing
`unsafe` blocks, so an access is gated by the innermost context rather than by
whether an `unsafe` appears anywhere in the function.

A `const` or `static` initializer and every `const func` body are constant
contexts. Besides the `const func` restrictions below, such a context cannot
call a non-`const` function (`E4011`), use dynamic dispatch (`E4016`) or a custom
iteration/indexing protocol (`E4017`), use an intrinsic outside the const
whitelist (`E4022`), write a static (`E4015`), or read a mutable static
(`E4026`). A constant value cannot contain `*mut T` (`E4012`). The checks run at
the use site during type checking, before constant evaluation.

A block-scope declaration is a statement and follows lexical scope:

```seal
func example() -> i32 {
    const SCALE: i32 = 3;
    static CALLS: i32 = 0;

    SCALE * 2 + CALLS
}
```

A block-scope `const` is visible from its declaration to the end of the block and
is not visible in its own initializer. Both local forms are evaluated
compile-time values: their type and initializer cannot capture the enclosing
function's parameters or bindings, and cannot mention its type parameters or
`Self` (`E3060`). A block-scope `static` has a single allocation shared by every
call and every instantiation of the enclosing function. A local `const func`
does not exist.

`const func` declares a function that may run at compile time and at runtime. It
may declare type parameters, but a compile-time call must supply concrete type
arguments. Like any function it has a signature; a `const` or `static` body
also has one, with no parameters and the declared type as its return type, so
consumers never read an initializer outside its body. A `const func` signature
and body cannot mention `*mut` (`E3062`), call a
non-`const` function (`E4018`), use dynamic dispatch or a trait-bound call
(`E4021`), or access a mutable static (`E4020`). Ordinary functions can call a
`const func` like any other function.

Constant evaluation is bounded and deterministic. It reports a typed diagnostic
for division or remainder by zero (`E4002`, `E4003`), signed division overflow
(`E4004`), out-of-bounds indexing and slicing (`E4005`, `E4006`), dependency
cycles (`E4007`), step, depth, allocation, and size limits (`E4008`–`E4010`,
`E4025`), and forbidden operations in a constant context (`E4011`–`E4027`).

Three boundaries keep this section from growing into a type-system feature of
its own. There are no associated constants: `Type::VALUE` still resolves only
an enum variant or an associated function, so a constant never participates in
method resolution or in a trait implementation. Array lengths stay integer
literals, so a constant is never read during parsing or layout. And a `const`
item is not a type-level constant: `const N: usize = 4` cannot appear where a
type argument is expected. Const generics, associated constants, and
value-level array lengths are separate features with their own design.

A `const` or `static` may name a trait object, a slice, or a string through a
reference, because those pointees are `Sized` behind the pointer. A trait-object
static records the concrete type and principal trait reference, which is what
lets monomorphization collect the vtable its initializer points at; a static
never needs a runtime type search.

#### Compile-time calls

A call from a constant context to a `const func` is interpreted directly from
its MIR body. Arguments are evaluated in source order, the body runs with its
own frame, and `return` copies the return place into the caller. Direct
recursion and mutual calls are ordinary call-stack frames bounded by the depth
and step limits, not dependency cycles. A generic call substitutes the
instance's concrete type arguments into every local and rvalue type before
interpretation; a call that still mentions a type parameter or inference
variable is rejected (`E4027`) because no layout exists for it. A callee that
is not a `const func` is an error (`E4011` in an initializer, `E4018` in a
`const func`), and a trait-bound or dynamic call is likewise rejected (`E4016`,
`E4021`). Panics and limit failures include the compile-time call chain.

The intrinsic whitelist is closed and mirrors runtime code generation: `len`
and `is_empty`, `swap`/`reverse`/`fill` on local arrays and slices, every
integer and float method, `bool`/`char` conversions and predicates, and
`char::len_utf8`. An intrinsic outside the whitelist reports `E4022` during
type checking; the evaluator never guesses at an operation it cannot reproduce
exactly.

### Constant evaluation

The evaluator interprets each `const` and `static` initializer's MIR before
borrow checking. A value is a typed tree, never a byte blob: integers keep their
truncated width, floats keep their IEEE bits, and aggregates keep declaration
order. A reference is an allocation index plus a byte offset and the pointee
type; a slice adds its length and element type; a trait object adds its concrete
type and principal trait reference. Byte offsets and layouts never decide the
meaning of a value, because the evaluator walks the typed tree when it reads a
projected place.

Taking a reference to a temporary in a constant context preserves the value in
a compiler-owned promoted allocation, so `&CONST` and `&[1, 2][..]` have program
lifetime. Taking a reference to a binding instead points at the initializer's
frame allocation; if the resulting value escapes, evaluation fails with
`E4013`, because that storage dies with the frame. A static's allocation exists
before its initializer runs, which is what makes address-only cycles between
statics (`A` points at `B` and `B` points back at `A`) legal. Reading a value
from an item still being evaluated is a value cycle and reports `E4007` with the
chain of item names. Limits and step counting are deterministic, so the same
program always produces the same diagnostics and the same allocation order.

Array lengths remain integer literals. `const N: usize = 4; [i32; N]` is not part
of this feature; const generics and type-level constants are a separate gap.
Associated constants (`Type::VALUE`) and `extern` statics are also outside this
feature.

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
order. A struct owns its fields. It is `Copy` only after an explicit
`impl Copy for Point {}` that covers the declaration, and only when every field
is `Copy`; without that implementation, assignment, argument passing, and
return move it. A `Copy` struct is duplicated bitwise by those operations.
Reading a `Copy` field copies only that field. The implementation is validated
once during HIR lowering and is not a layout or ABI change.
Fields initialize independently: `let mut p: Point; p.x = 1; p.y = 2;` is valid,
reading `p.x` before assigning it is an error, and a whole assignment initializes
every field at once. A field access on a reference requires an explicit
dereference and does not move out of the reference.

An aggregate may contain itself only behind a reference. A recursive field by
value has no finite size and is rejected during type checking before layout
runs.

## Enums

```seal
enum Shape {
  Empty,
  Circle { radius: i32 },
  Ring { inner: i32, width: i32 },
  Dot(i32, i32),
}

func unit() -> Shape {
  Shape::Empty
}

func circle() -> Shape {
  Shape::Circle { radius: 3 }
}

func dot() -> Shape {
  Shape::Dot(3, 4)
}
```

An enum is a **nominal sum type**: a value is exactly one variant. A variant
carries no fields (a *unit* variant, written `Enum::Variant`), positional fields
(written `Enum::Variant(T, U)`), or named fields (written
`Enum::Variant { field: value, ... }`). The three forms are exclusive: a variant
cannot mix positional and named fields. Positional fields are written as types
only and bind by position; named fields are resolved by name. `Variant()` is not
a fourth form: an empty positional list is a unit variant and is rejected
(`E2039`), so the canonical spelling stays the bare path `Variant`. Two enums
with the same variants are distinct types, and a variant name is only meaningful
through its enum.

Construction initializes the discriminant and every field of the chosen variant
exactly once. An unknown variant is `E2032`, a struct used with `::Variant` is
`E2031`, an enum used as a struct literal is `E2035`, a unit variant given fields
is `E2033`, and a payload variant written as a bare path is `E2034`. A positional
variant written with braces is `E2036`, a named variant called with parentheses
is `E2037`, the wrong number of positional arguments is `E2038`, and a unit
variant called with `()` is `E2039`. A malformed declaration field list is
`E1010`. Named-field mistakes reuse the struct codes: an unknown field is
`E2007`, a repeated field is `E2008`, and a missing field is `E2009`. Duplicate
variant and field names are rejected at declaration (`E2029`, `E2006`).

Variants and associated functions share the `Type::name` namespace, so a variant
may not have the same name as an inherent or trait associated function of its
enum (`E2030`). An `impl` block may target an enum, which is how an associated
constructor such as `Shape::unit()` is declared; methods receive `self`exactly as on a struct, and a trait may be implemented for an enum. A method reads
the payload with `match` and can construct and return variants.

An enum owns its payload. It is `Copy` only after an explicit
`impl Copy for Shape {}` that covers the declaration, and only when every field
of every variant is `Copy`; without that implementation, assignment, argument
passing, and return move the whole value. A `Copy` enum is duplicated bitwise by
those operations. It is `Sized` when every field of every variant is; an unsized
variant field is `E3010`, and a variant field that makes the enum recursive by
value is `E3016`. A raw pointer field breaks the recursion because the pointer
itself is `Copy`. Moving a payload variant moves its fields as a unit; `match`
performs the only partial move out of a non-`Copy` variant, by binding one of its
fields.

## Patterns and `match`

`match` is an expression. It evaluates its scrutinee once into an internal
temporary and then tries the arms in source order; the first arm whose pattern
matches and whose guard succeeds supplies the result, and every arm must have
that result type. A `match` whose arms never fall through has the `never` type.

```seal
enum Command {
  Stop,
  Move(i32),
  Write { code: i32 },
}

func classify(command: Command) -> i32 {
  match command {
    Command::Stop => 0,
    Command::Move(step) if step > 10 => 2,
    Command::Move(step) => step,
    Command::Write { code } => code,
  }
}
```

| Pattern | Matches |
|---------|---------|
| `_` | any value, binding nothing |
| `name` | any value, binding it under `name` |
| `0`, `-1`, `2.5`, `'x'`, `true` | a scalar equal to the literal |
| `Enum::Variant` | a unit variant |
| `Enum::Variant(p, q)` | a positional variant, one pattern per field |
| `Enum::Variant { field: p, other }` | a named variant; a bare name is shorthand for `other: other` |
| `Struct { field: p, other, .. }` | a struct; a bare name is shorthand for `other: other`, and `..` ignores the remaining fields |
| `(p, q)`, `()` | a tuple; one pattern per element, in positional order |
| `a..=b`, `a..b` | an integer or `char` value inside the range |
| `name @ p` | any value matching `p`, also bound as `name` |
| `[p, q]`, `[first, .., last]` | a fixed-size array; `..` skips the elements between the fixed patterns |
| the same forms on `*[T]` | a slice; without `..` the pattern tests the exact runtime length |
| `p \| q` | either alternative |

A pattern name is always a binding: an enum variant is written with its enum
(`Mode::On`), never bare. Strings are not patterns (`E1011`), because comparing
borrowed text would need a runtime operation the language does not define. A
name may be bound once per pattern (`E2040`), and every alternative of `|` must
bind the same names, because the arm body runs once for all of them (`E2041`). A
pattern mirrors the declaration: fields cannot be written on a unit
variant (`E2042`), a positional variant needs exactly one pattern per field
(`E2043`), and a named pattern must use declared field names, written in any
order (`E2044`). A struct pattern has the same rules with its own codes: an
unknown field is `E2046`, a field written twice is `E2047`, and an omitted field
is `E2048` unless the pattern ends with `..`, which ignores every field it did
not name. A named variant follows the same rule (`E2043` without `..`). An
array pattern writes one pattern per element, or uses a single
`..` to skip the elements between its fixed patterns; a pattern with more fixed
elements than the array has is `E2049`. A tuple pattern writes exactly one
subpattern per element and has no rest form, so a different arity is `E3037`. On a slice reference (`*[T]` or
`*mut [T]`) the same forms test a runtime length: without `..` the length must
match exactly, and with `..` it must be at least the number of fixed patterns.
The length itself is not part of the type, so coverage partitions the length
domain: `[]`, `[only]`, and `[first, .., last]` together are exhaustive, and a
missing length is reported with a slice witness. Binding an element moves it,
so a slice whose element type is not `Copy` can only be matched by literals and
`_` (`E3018`). A range whose start is past its end matches nothing (`E2050`). Lowering reorders
named fields into declaration order, so every later phase sees one canonical
shape. An `@` binding is a binding with a subpattern: the subpattern must match
before the name is stored, and the name is available in the arm body and guard
under the same rules as any other binding.

A struct, tuple, or variant pattern also matches through any number of
references (`*T`, `*mut T`): the pattern reads the tag and the fields or
elements through the reference, so a field or element bound by name must be
`Copy`, and binding a non-`Copy` field or element is `E3018`. Literal, range, and binding patterns still match the reference
value itself, so dereference explicitly when the value you compare is behind it.

A guard is an `if` expression after the pattern. It must have type `bool`
(`E3003`) and may read only bindings whose type is `Copy` (`E2045`): a failed
guard continues with the next arm, so a value the guard moved could not be put
back. Bindings are materialized only after the whole pattern matched; `Copy`
bindings are assigned before the guard and the rest after it succeeds.

Coverage uses the usefulness algorithm from the pattern literature: an arm is
reachable when some value matches it and no earlier unguarded arm, and a match is
exhaustive when no value escapes every arm. `bool` and enums have finitely many
constructors, so covering every variant is enough; a struct, tuple, or array is
a single constructor, so its fields or elements decide coverage, and an unnamed
field left by `..` is
a wildcard for that purpose. Integer and character ranges
partition the value domain at every observed endpoint, which makes adjacent
ranges exhaustive without a catch-all and reports an uncovered gap as a concrete
range such as `10..=19`. Floats have no usable constructor order, so a float
match needs `_` or a binding. A guard never contributes coverage. A missing
value is `E3032` and names a witness such as `Mode::Off`; an arm that no value
can reach is `E3033`.

Matching a non-`Copy` scrutinee moves it into the temporary. A `Copy` scrutinee
is read and remains usable in the enclosing scope. Binding a field of a variant
moves that field when its type is not `Copy`, which is the one way to move part
of an enum out; the rest of the scrutinee is then unusable. Binding a `Copy`
field only copies it. `if let` and `while let` follow the same rule: a failed
match leaves a `Copy` scrutinee available, and a successful `Copy` binding does
not consume it. Reading a variant field is valid only after the discriminant
proved that variant active, which is why the projection exists only inside an
arm.

### Conditional `if let`

`if let pattern = scrutinee { ... } else { ... }` is an expression that tests
one refutable pattern. The scrutinee is evaluated once, exactly as in `match`;
when the pattern matches, its bindings are initialized for the then block only.
The else block starts from the state before the pattern, so those bindings are
not visible there. An `else if let` chain is represented like an ordinary
`else if`: the else block's tail is the nested conditional.

```seal
enum Option {
  None,
  Some(i32),
}

func value_or(option: Option, fallback: i32) -> i32 {
  if let Option::Some(inner) = option {
    inner
  } else {
    fallback
  }
}
```

The pattern follows the same rules as a match arm, including struct, variant,
array, slice, reference, range, binding, `@`, and `|` patterns. It may be
refutable, so no exhaustiveness check applies. Without `else`, the expression
has type `()` and the then branch must also produce `()`; with `else`, both
branches must agree on the result type. Matching moves a non-`Copy` field into
its binding exactly as a match arm does, which means the scrutinee is partially
or wholly moved afterward. A pattern on a reference follows the reference and
can bind only fields whose type is `Copy` (`E3018`).

## Tuples

A tuple is a **structural** product type: its identity is the ordered list of
element types, so `(i32, bool)` written in two places is the same type. It is
not nominal and has no declaration, name, or fields. Tuples are written with
parentheses and commas:

```seal
let pair: (i32, bool) = (1, true);
let single: (i32,) = (7,);
let grouped = (pair.0);   // parentheses, not a tuple
let unit: () = ();
```

`(T)` groups a type and `(value)` groups an expression; a one-element tuple
requires the trailing comma. `()` is the unit type, not a distinct empty tuple,
so a tuple with no elements *is* `()`. There is no tuple rest pattern: a tuple
pattern writes exactly one pattern per element.

Elements are accessed by position with `.0`, `.1`, and so on. A tuple field is a
place, so it can be read, written, borrowed, moved, or projected further
(`nested.0.1`). Access requires a place base and does not autodereference a
reference; write `(*reference).0`. An index outside the arity is `E3036`, and
positional access on a non-tuple is `E3015`. Elements are evaluated in source
order, and construction initializes every element exactly once.

A tuple pattern is written `(p, q)` and follows the same rules as other
structural patterns: every element gets one subpattern, it may follow references
without an explicit dereference, and binding a non-`Copy` element through a
reference is `E3018`. A tuple is a single constructor for exhaustiveness, so
coverage depends on its element patterns.

The layout keeps declaration order with natural alignment and no reordering.
The size is the end of the last element rounded up to the tuple's alignment; a
one-element tuple has the same size and alignment as its element, and `()` is
zero-sized with alignment 1. A tuple is `Copy` when every element is, and
`Sized` when every element is. It has no destructor of its own; its elements
decide whether a future destructor is needed.

## Arrays

A fixed-size array is written `[T; N]`: one element type and one non-negative
length. The length is an integer literal, unsuffixed or suffixed `usize`, and
cannot exceed `u32::MAX` (LLVM's array length limit). The length is part of the
type: `[i32; 2]` and `[i32; 3]` never unify.

```seal
let mut values: [i32; 4] = [1, 2, 3, 4];
values[1] = 20;
let zeros = [0; 8];
let empty: [i32; 0] = [];
```

An array literal keeps source order. `[value; N]` evaluates `value` once and
copies it into every element, so the element type must be `Copy`; this is checked
even when `N` is zero. `[]` has no element from which to infer a type, so it needs
an expected array type (`E3019`). A literal whose element or length disagrees
with its annotation is a normal type mismatch.

Indexing is written `base[index]` and selects a place, so it can be read,
assigned, borrowed, or projected further. Arrays and slices keep built-in
indexing: the index type is exactly `usize`, an unsuffixed integer literal
adopts `usize`, a constant out-of-range array index is `E3022` at compile time,
and a runtime index is evaluated once and bounds-checked before the access. An
out-of-range value panics with status 101.

A custom base instead implements `Index<Key, Value>` for reads and
`IndexMut<Key, Value>` for writes, compound assignments, and exclusive borrows.
The index expression selects the `Key` type and must select exactly one
implementation (`E3053`); `Value` comes from that implementation or from the
expected type (`E3055`). Both methods return a reference into the base, so the
result is still a place and can be nested. A write without `IndexMut` is
`E3054`. A type with neither built-in indexing nor a matching protocol is
`E3021`, and indexing a value that is not a place is `E2013`.

A simple assignment evaluates its value before the target place, so
`grid[0] = grid[0] + 1` ends the shared read before `IndexMut` borrows the
base. A compound assignment or update evaluates the target place once first,
because it reads the old value through that place; the right-hand side follows.

The size is `N × size(T)` and the alignment is `align(T)`. A zero-length array
occupies no bytes but keeps the element alignment. Nested arrays multiply their
sizes. A type whose size overflows or exceeds the target address space is
`E3023`. Arrays derive `Copy`, `Sized`, and `Drop` from their element: an array
of references is `Copy`, an array of structs without a `Copy` implementation is
not, and an array of `str` is unsized.

Initialization is tracked per element. `let mut a: [i32; 2]; a[0] = 1; a[1] = 2;`
initializes the array field by field, reading `a[0]` before its assignment is an
error, and a whole assignment initializes every element at once. A write through
a runtime index requires the whole array to be initialized, because the compiler
cannot prove which element remains untouched.

## Slices

A slice is an unsized view over a contiguous sequence: it is written `[T]` and
only appears behind a reference. `*[T]` and `*mut [T]` are fat pointers holding
the element address and the element count, so the length is available in O(1)
without a terminator or a second parameter.

```seal
func sum(values: *[i32]) -> i32 {
  let mut index: usize = 0;
  let mut total = 0;
  while index < len(values) {
    total = total + values[index];
    index = index + 1;
  }
  total
}

func main() -> i32 {
  let values = [1, 2, 3, 4];
  let all: *[i32] = &values;       // array-to-slice coercion
  let middle = &values[1..3];      // bounded range
  let tail = &values[2..];         // open end
  let head = &values[..2];         // open start
  sum(all) + sum(middle) + sum(tail) + sum(head)
}
```

A range is written `base[start..end]`, with either bound optional. It denotes an
unsized place; it must be borrowed, and the borrow creates the fat reference.
Both bounds are `usize`. For an array, literal bounds are checked against the
length at compile time (`E3022`); a start greater than the end is `E3026`. At
run time, `start <= end <= length` is checked once, before the reference is
built, and a violation panics with status 101. A full `[..]` range needs no
check. Ranges work on arrays and on existing slice references, so `&tail[1..]`
reborrows a smaller view.

An array reference coerces to a slice reference when the expected type requires
one: `let s: *[i32] = &values;` accepts `&[i32; N]`, while `let r = &values;`
keeps the full array type. Slicing `&mut values[..]` requires a writable source:
a `mut` binding, an array field of one, or a `*mut [T]` reference (`E3017`).
Indexing a slice reference auto-dereferences it, so `values[index]` works for
`*[T]` and `*mut [T]`; the index is evaluated once and checked against the
runtime length before the access. Reading `*slice` as a value is rejected
because `[T]` is unsized (`E3024`), and a bare range is rejected for the same
reason.

`len(slice)` is the built-in length operator. It takes exactly one `*[T]` or
`*mut [T]` and returns `usize`; other types are `E3025`, and the wrong argument
count is `E2014`. A user function named `len` shadows the built-in. `slice.len()`
is the method form and returns the same value from the fat pointer's length
member. A slice is neither `Copy` nor `Sized` nor owning: the reference is
`Copy`, and creating or copying it extends the loan of the underlying storage
like any other reference.

## Iteration and indexing protocols

Seal installs four compiler-owned traits. They cannot be redeclared (`E2066`),
and an implementation cannot target an array, slice, or reference to one
(`E2067`): built-in sequences keep their bounds-checked behavior.

```text
IntoIterator<Iter, Item> { func iter(self: Self) -> Iter; }
Iterator<Item> { func has_next(self: *Self) -> bool; func next(self: *mut Self) -> Item; }
Index<Key, Value> { func index(self: *Self, key: Key) -> *Value; }
IndexMut<Key, Value> { func index_mut(self: *mut Self, key: Key) -> *mut Value; }
```

A `for` loop tries the built-in ranges, arrays, and slices first. For another
source `S`, it peels reference layers and looks for `S: IntoIterator<Iter,
Item>`; if there is none, it accepts `S: Iterator<Item>` directly. When both are
available, `IntoIterator` wins. The cursor returned by `iter` must implement
`Iterator<Item>` with the same item type; otherwise the loop is `E3051`. More
than one `IntoIterator` or `Iterator` implementation is ambiguous (`E3050`,
`E3052`), and a type that leaves a protocol argument unresolved is `E3055`.

The source is evaluated once. Its impl may take it by value, by shared
reference, or by exclusive reference, so `for value in &collection` and
`for value in &mut collection` are ordinary protocol selections. The cursor is
stored for the loop's lifetime. `has_next` does not mutate it, `next` advances
it, and the item is moved into the pattern. A non-`Copy` item therefore moves
once per iteration, and a refutable pattern filters elements according to the
normal `for` pattern rules. `break` leaves the loop, while `continue` returns to
`has_next` without calling `next` twice.

Custom indexing is selected by the shape of the place. Reads use `Index`;
writes, compound assignments, increment/decrement, and exclusive borrows use
`IndexMut`. The key expression is evaluated once and its type selects the
implementation, while the expected value type or the implementation fixes
`Value`. `Index` and `IndexMut` are independent: implementing one does not
provide the other. Unlike built-in arrays and slices, the compiler does not add
a bounds check to a custom implementation; the implementation owns the
validity and lifetime of the reference it returns. A returned reference keeps
the base loan alive through the ordinary call and borrow rules.

Protocol methods are ordinary trait methods. Static calls are direct after
monomorphization, and `*dyn Index<Key, Value>` or `*mut dyn IndexMut<Key,
Value>` uses the existing vtable dispatch. `IntoIterator` is not dyn-compatible
because `iter` takes `self` by value, so forming `dyn IntoIterator` is `E2065`
before monomorphization.

## The `Drop` trait

`Drop` is the compiler-owned trait that decides what happens when a value's
scope ends. It is installed like the protocols and cannot be redeclared
(`E2066`):

```text
Drop { func drop(self: *mut Self); }
```

The receiver is `*mut Self`, the method takes no other parameter, and the return
type is `()`; any other shape is `E2024`. An implementation must cover the whole
declaration: `impl Drop for Pair<T>` is valid, while a specialized
`impl Drop for Pair<i32>` and a second implementation for the same type are
`E3067`. A `Copy` type cannot implement `Drop` (`E3068`), because assignment
would duplicate the value and its destructor, and a `const func drop` is
`E3069`, because a destructor only runs while the program runs.

The trait is not a requirement a generic function can ask for: a bound or a
supertrait naming `Drop` is `E3064`, and `*dyn Drop` is `E3066`. The destructor
is not part of the callable interface either: `value.drop()` and
`Drop::drop(&mut value)` are `E3065`, because the compiler has already decided
when the value is released.

The built-in `drop(value)` releases a value early. It returns `()` and consumes
its operand exactly like a move: a non-`Copy` place is moved, so the destructor
runs at the call and any later use is `E5005`, while a `Copy` value, a
reference, or a raw pointer is accepted and does nothing. Moving a field out of
a type that implements `Drop` stays `E5014`. The name is not reserved: a user
function or binding named `drop` shadows the built-in, as with `len`.

A validated implementation is recorded on the type context. The drop glue of
the concrete type calls the destructor first and then drops the fields that need
dropping, in the order the memory model defines. Generic code uses the
destructor without naming it: monomorphization instantiates the recorded method
for each concrete argument list, and code generation calls that instance from
the glue.

A `static` is initialized before the program starts and is never released, so
its destructor never runs; only a value a scope owns is dropped. Constant
evaluation has no scope to leave either, so a constant initializer or a
`const func` that drops a value whose type declares a destructor is `E4028`.
Constructing such a value and initializing a `const` or `static` with it, or
returning it from a `const func`, is allowed.

## Methods

A method call has the postfix form `.name(arguments)` and resolves from the
receiver's type. On a struct or `str`, an inherent method declared in an `impl`
block is tried first; on scalars, sequences, strings, and references to them,
lookup uses the closed compiler-owned intrinsic catalog. Every reference layer
is followed before lookup, so `(*reference).abs()` and `reference.abs()` resolve
to the same scalar method, and a `*Point` can call a `Point` method written for
`*Point`.
There is no method value. A call on a nominal type, a type parameter, or an
intrinsic receiver is static: a trait method on a concrete type is a direct call
to the implementing function or to the trait's default body when the impl omits
it, and a method declared by a trait bound records the trait method until
monomorphization selects the implementation or default. A call through
`*dyn Trait` or `*mut dyn Trait` is dynamic and loads the function from the
receiver's vtable, as described under trait objects below. Methods and impls may
declare type parameters in both cases. An unknown name is `E3027`, an associated
function written as a method is `E2019`, and a method written as `Type::name` is
`E2019` as well.

```seal
func main() -> i32 {
  let mut values = [3, -1, 2];
  values.reverse();
  values.fill(7);
  (-5).abs().clamp(0, 3) + values.len().to_i32()
}
```

A method call evaluates its receiver first, then its arguments left to right, and
then performs the operation. Arguments must match the selected signature
exactly; the wrong count is `E3028`. Methods chain without temporaries in the
syntax, and an array, slice, or struct expression may be a temporary receiver. A
modifying method requires a writable place: a `mut` binding, a field reached
through one, or a `*mut T` reference. A shared reference, a shared slice, or an
immutable binding is `E3029`. Calling a modifying method on a temporary is
accepted because the temporary's storage does not outlive the call.

### User-defined inherent methods

An `impl` block attaches functions to one nominal struct declared in the same
module. A function whose first parameter is named `self` is an instance method;
its receiver type is written explicitly and must be the struct, `*Struct`, or
`*mut Struct`. A function without `self` is associated and is called through the
type with `Struct::name(...)`.

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Point {
  func new(x: i32, y: i32) -> Point {
    Point { x: x, y: y }
  }

  func sum(self: *Point) -> i32 {
    (*self).x + (*self).y
  }

  func shift(self: *mut Point, dx: i32, dy: i32) {
    (*self).x += dx;
    (*self).y += dy;
  }
}

func main() -> i32 {
  let mut point = Point::new(19, 22);
  point.shift(1, 2);
  point.sum()
}
```

A value receiver (`self: Point`) moves or copies the receiver as an ordinary
argument. A shared receiver (`self: *Point`) borrows it shared; an exclusive
receiver (`self: *mut Point`) borrows it exclusively and requires a writable
place. When the receiver expression already has the declared reference type it
is passed unchanged; otherwise a receiver of the struct type is borrowed
automatically. A receiver of another type is `E3031`.

Methods and associated functions share one namespace per type, so two impls may
not define the same name and a function may not be both associated and an
instance method (`E2017`). Different types may use the same method names; the
receiver's type selects the impl, and the generated symbol includes the type.
`self` must be the first parameter (`E2016`), and the impl target must be a
struct (`E2015`); the compiler-owned `core::str` module is the reserved
exception that supplies the inherent string surface. The ordinary borrow checker
sees the automatic receiver borrow as a real loan, so a live conflicting loan is
`E5008`.

### Traits and static dispatch

A trait declares a set of methods, each with a signature and optionally a
default body. `Self` in a signature stands for the implementing struct,
including in the receiver, so one declaration describes every implementor.
`impl Trait for Struct` supplies a function for each required method; a method
with a default body may be omitted, and the implementation then inherits that
body.

```seal
struct Circle {
  radius: i32,
}

trait Shape {
  func area(self: *Self) -> i32;
  func unit() -> i32;
}

impl Shape for Circle {
  func area(self: *Circle) -> i32 {
    (*self).radius * (*self).radius
  }

  func unit() -> i32 {
    1
  }
}

func main() -> i32 {
  let circle = Circle { radius: 3 };
  circle.area() + Circle::unit()
}
```

Traits and impls may declare type parameters. An impl header is matched
structurally: `impl<K> Keyed<K> for Pair<K, K>` applies to `Pair<i32, i32>`, and
`impl<T> Keyed<i32> for Pair<T, i32>` fixes the trait argument while leaving the
self argument open. An impl may also target a sized primitive, array, tuple, or
reference; only inherent impls are restricted to a struct or enum (`E2056`).

A trait may require other traits with `trait Sub: Super` (or in its `where`
clause). Every `T: Sub` therefore also proves `Super`, and the requirement is
transitive. A cycle in that relation is rejected (`E2064`), and a duplicate
requirement is kept once.

Lookup is ordered: an inherent method on the struct or `str` wins, then the
bounds visible on a type parameter, then the trait methods implemented for a
concrete type, and finally the intrinsic catalog for receivers with no user
method. For `str`, the registered inherent methods are checked before trait
impls and the catalog. A concrete candidate is either the function an impl
supplies or, when the impl omits a method that has a default body, that default
body, so a static trait call is a direct call with no vtable involved. A call through a type parameter
has no known impl yet; it records the trait method and its arguments for
monomorphization instead. When two implemented traits declare the same method
name and nothing earlier resolves it, the call is ambiguous (`E2028`). Seal has
no syntax for naming the trait at a call, so the fix is to rename one of the
methods; an inherent method with the name resolves the call instead.

A default body is checked once, with `Self` as a parameter and with the
implicit `Self: Trait` bound. It may call required methods, other defaults, and
associated functions through `self`; those calls are symbolic and
monomorphization selects the override or the default for the concrete `Self`.
An impl that supplies a method with the same name overrides the default. The
override must have the same receiver, parameter types, and return type after
`Self` and the trait arguments are replaced (`E2024`).

```seal
struct Circle { radius: i32 }

trait Shape {
  func area(self: *Self) -> i32;
  func twice(self: *Self) -> i32 { self.area() * 2 }
  func unit() -> i32 { 1 }
}

impl Shape for Circle {
  func area(self: *Circle) -> i32 {
    (*self).radius * (*self).radius
  }
}

func main() -> i32 {
  let circle = Circle { radius: 3 };
  circle.twice() + Circle::unit()
}
```

Here `twice` dispatches to the override of `area`, and `Circle::unit()` uses
the default associated function because the impl omits it. `Self::unit()`
inside a default body is symbolic in the same way: it selects the override when
the concrete impl supplies one. An inherent associated function with the same
name still wins over the trait default. An associated function with a default
body is not dispatchable through `dyn Trait`; object safety still rejects every
associated function without a receiver (`E2065`).

Implementing a trait is checked as a complete contract. Every declared method
without a default body must be present (`E2022`), nothing else may appear in the
block (`E2023`), and the receiver, parameter types, and return type must match
the declaration after `Self` and the trait arguments are replaced (`E2024`). A
trait receiver uses one of the three `Self` forms (`E2016`), while an impl writes
the concrete type. A type implements a trait once: an identical header is
`E2021`, and two headers that can describe the same instance are rejected as
overlapping (`E2058`), with the unifying substitution in the message. Bounds on
an impl do not make it disjoint, so `impl<T: Copy> Trait for Pair<T, T>`
overlaps `impl<T> Trait for Pair<T, T>`. A trait name is neither a type nor a
value (`E2027`), and `Self` outside a trait signature is `E2025`.

A method that declares `self: *Struct` can also be called through a
`*mut Struct` receiver: the compiler reborrows it as `*Struct`, a shared loan of
the pointee, so the borrow checker sees the same access a written `&*reference`
would create. The conversion is one-way; a `*Struct` never reaches an exclusive
receiver (`E3031`). A trait may declare an associated function with no receiver,
which is called as `Struct::name()` like an inherent one and may also have a
default body.

### Trait objects and dynamic dispatch

`dyn Trait` is an unsized type denoting any value of a concrete type implementing
`Trait`. It can only appear behind a reference: `*dyn Trait` gives shared access
and `*mut dyn Trait` gives exclusive access. Both are `Copy` fat pointers of two
words, `{ data, vtable }`, with the data address first and the vtable address
second. The mutability is static in the reference kind; both forms share one
representation.

```seal
trait Shape {
  func area(self: *Self) -> i32;
}

struct Circle { radius: i32 }
struct Square { side: i32 }

impl Shape for Circle {
  func area(self: *Circle) -> i32 { (*self).radius * (*self).radius }
}

impl Shape for Square {
  func area(self: *Square) -> i32 { (*self).side * (*self).side }
}

func area(shape: *dyn Shape) -> i32 {
  shape.area()
}

func main() -> i32 {
  let circle = Circle { radius: 3 };
  let square = Square { side: 4 };
  area(&circle) + area(&square)
}
```

A trait argument after `dyn` must be fully written, so a generic trait object
spells its arguments (`dyn Convert<i32>`). Coercion happens at an expression
with an expected trait-object type: `*S` becomes `*dyn Trait` and `*mut S`
becomes `*mut dyn Trait` when `S` implements the trait. The reference kind is
preserved, so a `*mut S` does not implicitly become `*dyn Trait` (`E3047`); write
`&*pointer` to reborrow it as shared. A concrete type that does not implement
the trait is `E3046`. Coercion keeps the data address and records the original
loan, so it neither copies the value nor allocates.

A trait is usable as a trait object only when every method is dispatchable: a
receiver of `*Self` or `*mut Self`, no `Self` in any other parameter or return
type, no generic method type parameters, and no associated function. Violations
are `E2065` when the `dyn` type is formed, not when the trait is declared; the
same trait remains usable for static dispatch. Supertraits must themselves be
dispatchable, and their methods are part of the principal object's vtable.

Lookup on a trait object starts with the principal trait, then its supertraits
depth-first in declaration order, with duplicate methods kept once. A method
only a supertrait declares is callable through `*dyn Sub` because `Sub: Super`
proves it. Two traits that declare the same method and are not resolved earlier
make the call ambiguous (`E2028`). A method taking `self: *mut Self` requires a
`*mut dyn Trait` receiver; calling it through `*dyn Trait` is `E3029`. A shared
receiver called through `*mut dyn Trait` is reborrowed as shared, so the callee
cannot write while the shared loan is live.

Each `(concrete type, fully substituted trait reference)` pair has one vtable,
emitted as a constant in the program. Its layout is compiler-private, not a
stable external ABI:

```text
slot 0: drop_in_place  fn(*mut u8)
slot 1: size           usize
slot 2: align          usize
slot 3..: method pointers
```

Method slots follow the principal trait's methods in declaration order, then
supertrait methods depth-first in declaration order, deduplicated by method
identity on first occurrence. The slot is fixed at compile time and shared by
type checking, MIR, monomorphization, and code generation. A dynamic call loads
the slot from the vtable already embedded in the receiver; it never searches for
the concrete type at runtime. The drop slot names the concrete type's drop
glue, an internal function that runs the declared destructor and then drops the
fields; it is empty for a type that declares none. The size and alignment slots
complete the header. A reference to a trait object is still a borrowed view:
converting to it, copying it, and reborrowing it copy only the two pointer
words.

### Signed and unsigned integers

Both families share these methods; `Self` is the receiver's exact integer type.

| Method | Signature | Behavior |
|--------|-----------|----------|
| `min` | `(other: Self) -> Self` | Smaller value, signed comparison for `Int` |
| `max` | `(other: Self) -> Self` | Larger value, signed comparison for `Int` |
| `clamp` | `(min: Self, max: Self) -> Self` | Panics unless `min <= max` |
| `wrapping_add` | `(other: Self) -> Self` | Addition modulo 2^N |
| `wrapping_sub` | `(other: Self) -> Self` | Subtraction modulo 2^N |
| `wrapping_mul` | `(other: Self) -> Self` | Multiplication modulo 2^N |
| `saturating_add` | `(other: Self) -> Self` | Clamps overflow to the type's bounds |
| `saturating_sub` | `(other: Self) -> Self` | Clamps underflow to the type's bounds |
| `saturating_mul` | `(other: Self) -> Self` | Clamps overflow to the type's bounds |
| `count_ones` | `() -> u32` | Population count |
| `leading_zeros` | `() -> u32` | Zero bits before the highest set bit; the width for zero |
| `trailing_zeros` | `() -> u32` | Zero bits after the lowest set bit; the width for zero |
| `rotate_left` | `(amount: u32) -> Self` | Rotates left by `amount` modulo the bit width |
| `rotate_right` | `(amount: u32) -> Self` | Rotates right by `amount` modulo the bit width |
| `swap_bytes` | `() -> Self` | Reverses byte order; a no-op below 16 bits |

Signed integers add:

| Method | Signature | Behavior |
|--------|-----------|----------|
| `abs` | `() -> Self` | Absolute value; `MIN.abs()` returns `MIN` (wrapping) |
| `signum` | `() -> Self` | `-1`, `0`, or `1` according to the sign |

Integer methods receive the value by copy. No method can change its receiver.

### Floating point

`Self` is `f32` or `f64`; predicates return `bool` and every other method returns
`Self` unless noted.

| Method | Signature | Behavior |
|--------|-----------|----------|
| `abs` | `() -> Self` | IEEE absolute value |
| `min` | `(other: Self) -> Self` | Smaller value, choosing the non-NaN operand |
| `max` | `(other: Self) -> Self` | Larger value, choosing the non-NaN operand |
| `clamp` | `(min: Self, max: Self) -> Self` | Panics unless `min <= max` |
| `is_nan` | `() -> bool` | True for either NaN |
| `is_finite` | `() -> bool` | True for non-NaN, non-infinite values |
| `is_infinite` | `() -> bool` | True for either infinity |
| `sqrt` | `() -> Self` | Square root, following IEEE rounding |
| `floor` | `() -> Self` | Rounds toward negative infinity |
| `ceil` | `() -> Self` | Rounds toward positive infinity |
| `round` | `() -> Self` | Nearest integer, halfway away from zero |
| `trunc` | `() -> Self` | Rounds toward zero |
| `fract` | `() -> Self` | `self - trunc(self)` |
| `powi` | `(exponent: i32) -> Self` | Raises to an integer power |

NaN and infinity propagate according to IEEE semantics. `min` and `max` return
the non-NaN operand when exactly one is NaN and NaN when both are; `clamp` panics
when either bound is NaN because the required ordering is not provable.
`floor`, `ceil`, `round`, and `trunc` preserve NaN, infinity, and signed zero.

### Boolean and character

| Owner | Method | Signature | Behavior |
|-------|--------|-----------|----------|
| `bool` | `not` | `() -> bool` | Logical negation |
| `bool` | `to_i32` | `() -> i32` | `false` becomes `0`, `true` becomes `1` |
| `bool` | `toggle` | `() -> ()` | Flips the receiver in place; requires an exclusive receiver |
| `char` | `is_ascii` | `() -> bool` | True for `U+0000..=U+007F` |
| `char` | `is_ascii_alphabetic` | `() -> bool` | True for `A..=Z` and `a..=z` |
| `char` | `is_ascii_digit` | `() -> bool` | True for `0..=9` |
| `char` | `is_ascii_whitespace` | `() -> bool` | Space, tab, newline, form feed, or carriage return |
| `char` | `to_ascii_uppercase` | `() -> char` | Uppercases ASCII letters, leaves every other char unchanged |
| `char` | `to_ascii_lowercase` | `() -> char` | Lowercases ASCII letters, leaves every other char unchanged |
| `char` | `len_utf8` | `() -> usize` | UTF-8 encoding length, from 1 through 4 |
| `char` | `to_u32` | `() -> u32` | The Unicode scalar value |

`toggle` is the only scalar method with an observable effect. It writes through
its receiver, so the borrow checker sees an exclusive access to that place: no
shared loan may be live at the call.

### Arrays and slices

| Method | Signature | Behavior |
|--------|-----------|----------|
| `len` | `() -> usize` | Number of elements; O(1) |
| `is_empty` | `() -> bool` | True when `len() == 0` |
| `swap` | `(first: usize, second: usize) -> ()` | Swaps two elements; panics if either index is out of range |
| `reverse` | `() -> ()` | Reverses the elements in place; accepts an empty sequence |
| `fill` | `(value: Element) -> ()` | Copies `value` into every element; requires `Element: Copy` |

Arrays use the array's compile-time length; slices read the runtime length from
the fat pointer. `len` and `is_empty` borrow the receiver shared, while `swap`,
`reverse`, and `fill` borrow it exclusively. For an array, the receiver may be a
`mut` binding or reach the array through a `*mut [T; N]`. For a slice, modifying
methods require `*mut [T]`; a `*[T]` is `E3029`. `fill` on a non-`Copy` element
is `E3030` even when the length is zero.

`swap` checks both indices before either write, so a panic never leaves one
element overwritten. `reverse` and `fill` are loops over the sequence and do no
allocation; an empty sequence performs no operation. Indexing, bounds checks,
and temporary storage follow the array and slice rules above, and the borrow
checker tracks the receiver's storage for the duration of the call.

## Unit, never, and internal types

`()` is the unit type: it is the result of a function with no `->` clause, the
empty tuple literal, and the type written `()` in a signature or annotation.
`!` is the never type, the type of expressions that do not return, such as
`return`, `break`, and `continue`; it has no source spelling in type position in
v0.1. Both are zero-sized, aligned to 1, `Copy`, and `Sized`.

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
| `Copy` | Scalars, `()`, `!`, every reference, including `*dyn Trait` and `*mut dyn Trait`, and both raw pointer kinds are copied bitwise. An array is `Copy` when its element is; a tuple when every element is. A struct or enum is `Copy` only with an explicit `impl Copy` covering the declaration and every field of every variant; without it, the aggregate is not `Copy`, even when all fields are. `str`, slices, and `dyn Trait` are never `Copy`. |
| `Sized` | Every type has a compile-time size except `str`, slices, `dyn Trait`, and an unresolved `Infer`. A struct is sized when every field is; an enum when every field of every variant is; an array when its element is; a tuple when every element is. |
| `needs_drop` | An aggregate with a declared destructor answers `true`; otherwise an array, tuple, struct, and enum derive the answer from their elements or fields. A trait object's vtable points its drop slot at the concrete type's glue, which runs the destructor and then drops the parts that need it. |

`Copy` and a destructor are mutually exclusive (`E3068`), and there is no
user-defined copy hook. An opt-in implementation is validated once during HIR
lowering, recorded in `TyCtxt`, and the query derives the answer from the
registry plus every field and variant. It does not change layout or ABI.

## Equality

`==` and `!=` compare values, never addresses, and they read both operands
without moving them: `a == b` leaves `a` and `b` usable even when the type is
not `Copy`.

| Type | Comparison rule |
|------|-----------------|
| integers, `char`, `bool` | Bit equality of the same value, so `==` agrees with the ordering operators |
| `f32`, `f64` | IEEE 754: `-0.0 == 0.0` is true and `NaN == NaN` is false |
| `()` | Always equal |
| arrays, tuples | Element by element, in order |
| structs | Field by field, in declaration order |
| enums | Equal when both hold the same variant and that variant's fields are equal |
| `*str` | Length, then bytes |
| `*[T]` and `*mut [T]` | Length, then elements; `T` must itself be comparable |
| `*T`, `*mut T` | Rejected: only the address could differ, which is not value equality |
| `*dyn Trait` | Rejected: the concrete type is erased, so there is no structural comparison |
| `func(...)` | Rejected: there is no value equality for a function pointer |
| closures | Rejected: a closure type is not part of the comparable surface |

A struct, tuple, or array is comparable only when every field or element is. A
nested aggregate recurses with the same rules, so a struct holding a `*[i32]`
compares the slice contents while a struct holding a `*Point` has no equality.
Type parameters have none either: comparing an operand of type `T` reports
`E3041`, and comparing an aggregate that contains a `T` reports `E3002`. There
is no operator overloading — a type either follows the rules above or cannot be
compared.

Reading rather than moving is what makes the operand forms useful: `*left ==
*right` compares the pointees of two thin references, and `value == ORIGIN`
compares against a non-`Copy` static. Neither is a move, so neither reports
`E3018` or `E3058`. The same dereference or static read anywhere else is still a
move and is still rejected.

Constant evaluation implements exactly these rules, so an equality used in a
`const` or `static` initializer and the same comparison at runtime agree, NaN
and `-0.0` included.

## Layout

Layout is deterministic and defined by the language, not by the host C ABI:

1. Scalars use natural alignment: alignment equals size. `bool` is 1 byte,
   `char` is 4 bytes, and every integer and float has its width.
2. Every reference uses the target pointer width. `*str` is two words: data and
   length, both pointer-sized. `*dyn Trait` and `*mut dyn Trait` are two words:
   data and vtable, both pointer-sized.
3. Fields and tuple elements keep declaration order; there is no reordering,
   packing, or overlapping. Each field starts at the next multiple of its
   alignment.
4. A struct's alignment is the maximum of its fields' alignments, or 1 when it
   has no fields. Its size is the end of its last field rounded up to its own
   alignment, so trailing padding belongs to the value.
5. An enum starts with a tag: the smallest unsigned integer that addresses every
   variant (1 byte up to 256 variants, 2 bytes up to 65_536, 4 bytes above).
   The payload starts at the next multiple of the widest variant field alignment,
   is as large as the largest variant, and the whole value is padded to its
   maximum alignment. Variant field offsets are measured from the start of the
   enum, which is what code generation and debug information use.
6. A tuple is laid out exactly like a struct with positional fields: natural
alignment, no reordering, and trailing padding rounded to its alignment.
7. `()`, `!`, and `Error` are zero-sized with alignment 1. `str`, slices, and
   `dyn Trait` have no layout of their own.
8. An array has the element's alignment and `length × size(element)` bytes. It
   adds no padding between elements beyond the element's own size, which already
   includes its trailing padding.
9. A Seal function pointer is one pointer-sized word with pointer alignment.
10. A closure is laid out exactly like a struct whose fields are its captures in
    first-mention order, with natural alignment and trailing padding. An empty
    environment is a zero-sized value with alignment 1.

| Type | Size | Align | Offsets |
|------|-----:|------:|---------|
| `Point { x: i32, y: i32 }` | 8 | 4 | `x` at 0, `y` at 4 |
| `Mixed { flag: bool, value: i32, small: i8 }` | 12 | 4 | `flag` at 0, `value` at 4, `small` at 8 |
| `Segment { start: Point, end: Point }` | 16 | 4 | `start` at 0, `end` at 8 |
| `Empty {}` | 0 | 1 | none |
| `enum Number { Small { value: u8 }, Large { value: u64 } }` | 16 | 8 | tag at 0, payload at 8 |
| `(i32, bool)` | 8 | 4 | first at 0, second at 4 |
| `[i32; 4]` | 16 | 4 | elements at 0, 4, 8, 12 |
| `[u8; 0]` | 0 | 1 | none |
| `*i32` on a 64-bit target | 8 | 8 | none |
| `*str` on a 64-bit target | 16 | 8 | data at 0, length at 8 |
| `*dyn Shape` on a 64-bit target | 16 | 8 | data at 0, vtable at 8 |
| `func(i32) -> i32` on a 64-bit target | 8 | 8 | none |
| `\|x: i32\| x + 1` with no captures | 0 | 1 | none |
| `move \|\| token` holding `Token { value: i32 }` | 4 | 4 | capture at 0 |

Code generation materializes one stack slot per local, uses struct GEPs for
struct fields and tuple elements, indexes arrays with a bounds-checked byte
offset, and passes references as pointers — or as a `{ data, length }` pair for
`*str` and a `{ data, vtable }` pair for `*dyn Trait`. `()` and `!` are
zero-sized structs in the ABI.

## Loops

`while condition { ... }` repeats while the condition is `bool`. `for pattern
in source { ... }` evaluates `source` once and repeats for each element, with
`break` and `continue` acting on the innermost loop.

`while let pattern = scrutinee { ... }` reevaluates the scrutinee and retests
the pattern before every iteration. The pattern may be refutable, and it follows
the same rules as a match arm, including struct, variant, array, slice,
reference, range, binding, `@`, and `|` patterns. A successful match binds its
names only in the body; a failed match exits the loop. There is no `else`
branch. `continue` returns to the pattern test, so the scrutinee is evaluated
again, and `break` exits immediately. An irrefutable pattern repeats forever
unless the body reaches `break` or another diverging expression.

A `for` source is one of:

| Source | Element bound by the pattern |
|--------|------------------------------|
| `start..end`, `start..=end` | the integer counter |
| `[T; N]` | `T`; the array is moved unless it is `Copy`, in which case each item is copied and the source remains usable |
| `&[T; N]`, `&mut [T; N]` | `*T`, `*mut T` |
| `*[T]`, `*mut [T]` | `*T`, `*mut T` |
| `S: IntoIterator<I, T>` | `T`, after `S::iter` returns `I: Iterator<T>` |
| `S: Iterator<T>` | `T`, through the direct fallback |

The two bounds of a range must have the same integer type; an unsuffixed integer
literal on one side adopts the type of the other, so `0..len(values)` uses the
`usize` that `len` returns. Both bounds are evaluated once. The exclusive range
runs while `counter < end`, and the inclusive range while `counter <= end`;
the inclusive form exits before incrementing when the counter equals `end`, so
`0u8..=255u8` visits every value without overflowing.

For a built-in range or sequence the pattern must be irrefutable (`E3034`). A
custom protocol loop accepts a refutable pattern: an item that does not match is
skipped without binding, and the loop continues with the next `has_next` test.
The iterable is copied or moved once before the loop; a borrowed array or slice
is reborrowed element by element with the same shared or exclusive kind, so
writing through `*mut T` elements is allowed while the source is exclusively
borrowed. A custom source uses `IntoIterator` first and the direct `Iterator`
fallback second; the cursor's `has_next` tests whether another item exists, and
`next` moves it into the pattern. An unsupported source is `E3035`, ambiguous
protocols are `E3050`/`E3052`, and a cursor without the matching `Iterator` is
`E3051`. A `continue` in the body goes to the increment step or the next
protocol test, never directly past a pending `next`, so it always makes
progress.

## Growth

Every new type kind must define its layout, its `Copy`/`Sized`/`Drop` answers,
and its interning key before it can appear in a signature. Slices reuse the
element stride and add a runtime length; enum variants reuse the field, query,
and layout rules per variant; generic instantiations apply the same rules per
concrete argument;trait objects reuse the reference layout and add a vtable address; function
pointers reuse the pointer layout; closures reuse the aggregate layout over
their capture fields; owning heap types will add field-derived destructors.
None of these requires changing the identity, layout, or ownership model above:
`TyId` equality, declaration-order layout, and the move/borrow rules stay the
same.
