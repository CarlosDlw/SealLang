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
| Slice reference | `*[T]`, `*mut [T]` | 2 × pointer | pointer | yes | yes | no |
| Slice pointee | `[T]` | unsized | unsized | no | no | no |
| Aggregate | `S` | computed | computed | no | yes, if every field is | no |
| Enum | `E` | computed | computed | no | yes, if every variant field is | no |
| Array | `[T; N]` | N × size(T) | align(T) | yes, if `T` is | yes, if `T` is | no |
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
pointer, except `*str`, `*[T]`, and `*mut [T]`, which are two-word fat pointers
holding a data address and a length.

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
constructor such as `Shape::unit()` is declared; methods receive `self` exactly
as on a struct, and a trait may be implemented for an enum. A method cannot read
the payload until `match` exists, but it can construct and return variants.

An enum owns its payload and is never `Copy`, so assignment, argument passing,
and return move the whole value. It is `Sized` when every field of every variant
is; an unsized variant field is `E3010`, and a variant field that makes the enum
recursive by value is `E3016`. Moving a payload variant moves its fields as a
unit; there is no partial move out of a variant without `match`.

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
assigned, borrowed, or projected further. The index type is exactly `usize`;
an unsuffixed integer literal adopts `usize`. A constant out-of-range index is
`E3022` at compile time. A runtime index is evaluated once and bounds-checked
before the access; an out-of-range value panics with status 101. Indexing a
non-array is `E3021`, and indexing a value that is not a place is `E2013`.

The size is `N × size(T)` and the alignment is `align(T)`. A zero-length array
occupies no bytes but keeps the element alignment. Nested arrays multiply their
sizes. A type whose size overflows or exceeds the target address space is
`E3023`. Arrays derive `Copy`, `Sized`, and `Drop` from their element: an array
of references is `Copy`, an array of structs is not, and an array of `str` is
unsized.

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

## Methods

A method call has the postfix form `.name(arguments)` and resolves from the
receiver's type. On a struct, an inherent method declared in an `impl` block is
tried first; on scalars, sequences, and references to them, lookup uses the
closed compiler-owned intrinsic catalog. Every reference layer is followed
before lookup, so `(*reference).abs()` and `reference.abs()` resolve to the same
scalar method, and a `*Point` can call a `Point` method written for `*Point`.
There is no vtable, dynamic dispatch, method value, or generic method. A trait
method resolved on a nominal type is a static call to the implementing function.
An unknown name is `E3027`, an associated function written as a method is
`E2019`, and a method written as `Type::name` is `E2019` as well.

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
struct (`E2015`). The ordinary borrow checker sees the automatic receiver borrow
as a real loan, so a live conflicting loan is `E5008`.

### Traits and static dispatch

A trait declares a set of method signatures. `Self` in a signature stands for
the implementing struct, including in the receiver, so one declaration describes
every implementor. `impl Trait for Struct` supplies one function per signature.

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

Lookup is ordered: an inherent method on the struct wins, then the trait methods
the struct implements. Every candidate is the concrete function of one impl, so
a trait call is a direct call with no vtable, trait object, or dynamic dispatch.
When two implemented traits declare the same method name and the type has no
inherent method with that name, the call is ambiguous (`E2028`). Seal has no
syntax for naming the trait at a call, so the fix is to rename one of the
methods; an inherent method with the name resolves the call instead.

Implementing a trait is checked as a complete contract. Every declared method
must be present (`E2022`), nothing else may appear in the block (`E2023`), and
the receiver, parameter types, and return type must match the declaration after
`Self` is replaced with the struct (`E2024`). A trait receiver uses one of the
three `Self` forms (`E2016`), while an impl writes the concrete type. A type
implements a trait once (`E2021`), a trait name is neither a type nor a value
(`E2027`), and `Self` outside a trait signature is `E2025`.

A method that declares `self: *Struct` can also be called through a
`*mut Struct` receiver: the compiler reborrows it as `*Struct`, a shared loan of
the pointee, so the borrow checker sees the same access a written `&*reference`
would create. The conversion is one-way; a `*Struct` never reaches an exclusive
receiver (`E3031`). A trait may declare an associated function with no receiver,
which is called as `Struct::name()` like an inherent one.

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
| `Copy` | Scalars, `()`, `!`, and every reference are copied bitwise. An array is `Copy` when its element is. `str`, slices, and every struct or enum are not, even when a slice's elements are `Copy`. |
| `Sized` | Every type has a compile-time size except `str`, slices, and an unresolved `Infer`. A struct is sized when every field is; an enum when every field of every variant is; an array when its element is. |
| `needs_drop` | No current type runs a destructor. An array, struct, and enum derive the answer from their elements or fields once a type with a destructor exists. |

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
5. An enum starts with a tag: the smallest unsigned integer that addresses every
   variant (1 byte up to 256 variants, 2 bytes up to 65_536, 4 bytes above).
   The payload starts at the next multiple of the widest variant field alignment,
   is as large as the largest variant, and the whole value is padded to its
   maximum alignment. Variant field offsets are measured from the start of the
   enum, which is what code generation and debug information use.
6. `()`, `!`, and `Error` are zero-sized with alignment 1. `str` has no layout of
   its own.
7. An array has the element's alignment and `length × size(element)` bytes. It
   adds no padding between elements beyond the element's own size, which already
   includes its trailing padding.

| Type | Size | Align | Offsets |
|------|-----:|------:|---------|
| `Point { x: i32, y: i32 }` | 8 | 4 | `x` at 0, `y` at 4 |
| `Mixed { flag: bool, value: i32, small: i8 }` | 12 | 4 | `flag` at 0, `value` at 4, `small` at 8 |
| `Segment { start: Point, end: Point }` | 16 | 4 | `start` at 0, `end` at 8 |
| `Empty {}` | 0 | 1 | none |
| `enum Number { Small { value: u8 }, Large { value: u64 } }` | 16 | 8 | tag at 0, payload at 8 |
| `[i32; 4]` | 16 | 4 | elements at 0, 4, 8, 12 |
| `[u8; 0]` | 0 | 1 | none |
| `*i32` on a 64-bit target | 8 | 8 | none |
| `*str` on a 64-bit target | 16 | 8 | data at 0, length at 8 |

Code generation materializes one stack slot per local, uses struct GEPs for
fields, indexes arrays with a bounds-checked byte offset, and passes references
as pointers — or as a `{ data, length }` pair for `*str`. `()` and `!` are
zero-sized structs in the ABI.

## Growth

Every new type kind must define its layout, its `Copy`/`Sized`/`Drop` answers,
and its interning key before it can appear in a signature. Slices reuse the
element stride and add a runtime length; enum variants reuse the field, query,
and layout rules per variant; generics will instantiate the same rules per
concrete argument; owning heap types will add field-derived destructors. None of these requires changing the identity, layout, or ownership
model above: `TyId` equality, declaration-order layout, and the move/borrow
rules stay the same.
