# Opt-in `Copy`

Status: implemented in the current language version.

A `Copy` type is duplicated bitwise when it is assigned, passed by value, or
returned. Scalars, references, raw pointers, arrays, and tuples already have a
structural answer; a struct or enum becomes `Copy` only through an explicit
`impl Copy`, and only when every field of every variant is `Copy`.

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  let copy = point;
  drop(point);
  copy.x + point.y
}
```

The program returns `42`. Both bindings remain usable after the assignment and
after `drop`, because `Point` opted into `Copy`.

## 1. The mental model

Seal moves values by default. `Copy` changes only whether a whole-value transfer
duplicates the value or moves it; it does not introduce a deep copy,
allocation, reference counting, or a user-defined copy hook.

| Type | `Copy` | Rule |
|------|:------:|------|
| integers, floats, `bool`, `char`, `()`, `!` | yes | bitwise scalar |
| `*T`, `*mut T` | yes | the reference value is copied; its loan is copied too |
| `*raw T`, `*raw mut T` | yes | the address and metadata are copied; provenance is preserved |
| `[T; N]` | when `T` is `Copy` | every element is copied |
| `(T, U, …)` | when every element is `Copy` | every element is copied |
| `str`, `[T]`, `dyn Trait` | no | unsized values have no whole-value copy |
| `struct S` | only with `impl Copy for S` | every field must be `Copy` |
| `enum E` | only with `impl Copy for E` | every field of every variant must be `Copy` |

An aggregate without the implementation moves as a whole, even when all of its
fields happen to be `Copy`. This keeps ownership visible at nominal boundaries:
adding `impl Copy` is a deliberate declaration, not an inference from the
current field list.

`Copy` and `Drop` are mutually exclusive. A type that owns a destructor cannot
be duplicated, because both copies would claim the same release obligation
(`E3068`).

## 2. Declaring `Copy`

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

enum Direction {
  North,
  East,
}

impl Copy for Direction {}
```

The implementation must satisfy all of these rules:

- The target is a struct or enum declared in the current file. Primitives,
  references, raw pointers, tuples, arrays, `str`, slices, trait objects, and
  type parameters are not nominal targets (`E2015`).
- The implementation repeats the declaration's parameters in order. It cannot
  specialize an instantiation, omit a parameter, add a parameter, or reorder
  them (`E3076`).
- There is at most one implementation for a declaration. An exact duplicate is
  reported by coherence (`E2021` or `E2058`), not by `Copy` validation.
- Every field of every variant is `Copy`. A field that is not is `E3077` and the
  diagnostic names the field, or the field and variant, and its type.
- A type parameter used as a field needs a `Copy` bound. A reference or raw
  pointer field does not constrain its pointee; the pointer value is already
  `Copy`.
- A type containing itself by value is still `E3016`. Recursion through `*raw
  T`, `*T`, or `*mut T` is accepted because the pointer breaks the value cycle.
- The `impl Copy` block declares no functions. `Copy` has no members; a method
  or associated function in the block is `E2023`.

The implementation is validated once during HIR lowering. It is recorded next
to the aggregate in `TyCtxt`; later phases query the property instead of reading
the impl again. Validation registers candidates and removes those whose fields
are not `Copy` until it reaches a fixed point, so the declaration order of
related impls does not change the result.

An empty struct is `Copy` with an empty implementation:

```seal
struct Marker {}

impl Copy for Marker {}
```

## 3. Generic aggregates

A generic aggregate opts in once for the whole declaration. The ordinary form
uses a bound on the parameters that are stored by value:

```seal
struct Wrapper<T> {
  value: T,
}

impl<T: Copy> Copy for Wrapper<T> {}
```

`Wrapper<Point>` is `Copy` when `Point` is `Copy`; `Wrapper<Token>` is not when
`Token` has no implementation. Passing `Wrapper<Token>` to a `T: Copy` bound is
`E3043`, and using it in a body that requires a copy follows the same answer.

A `where` clause is equivalent:

```seal
struct Boxed<T> {
  value: T,
}

impl<T> Copy for Boxed<T> where T: Copy {}
```

Without the bound, a field of type `T` is `E3077` and the help adds the missing
`T: Copy`:

```seal
struct Bad<T> {
  value: T,
}

impl<T> Copy for Bad<T> {} // E3077: field `value` has type `T`
```

A raw pointer field needs no bound on its pointee:

```seal
struct Ptr<T> {
  value: *raw T,
}

impl<T> Copy for Ptr<T> {} // accepted
```

The same is true for `*T` and `*mut T` fields. The pointer itself is copied; the
pointee is not duplicated. A conditional implementation works inside a generic
body: a function with `T: Copy` may copy a `Wrapper<T>` without knowing the
concrete type.

A specialized implementation is not allowed:

```seal
struct Wrapper<T> {
  value: T,
}

impl Copy for Wrapper<i32> {} // E3076: does not cover the declaration
```

The rule prevents two instantiations of the same declaration from disagreeing
about whether they are `Copy`.

## 4. Fields and enum variants

Every field of every variant participates, including a unit variant's absence
of fields. This enum is `Copy` when `Point` is:

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

enum Shape {
  Empty,
  Holds(Point),
  Named { point: Point },
}

impl Copy for Shape {}
```

If one variant has a non-`Copy` field, the whole enum is rejected:

```seal
struct Token {
  value: i32,
}

enum Bad {
  Empty,
  Holds(Token),
}

impl Copy for Bad {} // E3077: field 0 of variant `Holds`
```

The diagnostic points at the implementation and names the first blocking field.
The other fields are not silently ignored; removing the first failure exposes
the next one on the next check.

Transitive validation works in both directions. An `Outer` whose field is
`Inner` is accepted when `Inner` has a valid implementation declared earlier or
later. An invalid chain reports one `E3077` for every implementation that
cannot become `Copy`; it never loops on a cycle.

## 5. `Copy` and `Drop`

`Copy` and `Drop` cannot hold for the same type, in either declaration order:

```seal
struct Point {
  x: i32,
}

impl Copy for Point {}
impl Drop for Point {
  func drop(self: *mut Point) {}
}
```

This is `E3068`. The diagnostic points at the `Copy` implementation, has a
secondary label at the destructor, and leaves `Drop` registered while the
`Copy` candidate is removed. A `Copy` type has no drop site, drop flag, or drop
glue.

A block cannot add a custom destructor to a `Copy` type after the fact. The
two properties are alternatives: a type is either duplicated with no cleanup,
or owned and released exactly once.

## 6. Moves, patterns, loops, and statics

### Assignment, arguments, and returns

An assignment or call reads the source value and stores a duplicate. The source
stays usable:

```seal
func pass(point: Point) -> Point {
  point
}

func main() -> i32 {
  let point = Point { x: 20, y: 2 };
  let assigned = point;
  let returned = pass(point);
  assigned.x + returned.y + point.x
}
```

The same rule applies to returning a `Copy` value and to reading a field or an
array element. A field read copies only that field, not the whole aggregate.

### Patterns and loops

A pattern binding of a `Copy` value copies it. The scrutinee remains usable
after the match, `if let`, or `while let`:

```seal
func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  let value = match point {
    Point { x, y } => x + y,
  };
  point.x + value
}
```

A failed `if let` over a `Copy` value leaves the value available, and a
successful `Copy` binding does not consume it. A guard may read a `Copy` binding;
the same binding is not available to a guard when its type is not `Copy`
(`E2045`).

An array indexed by value copies its element. `[value; N]` requires `value` to
be `Copy`. A by-value `for` loop copies each `Copy` item and leaves the source
array usable; a non-`Copy` array is consumed by the loop.

### `drop`, statics, and constants

`drop(copy_value)` consumes nothing and runs no destructor. The value remains
usable:

```seal
func main() -> i32 {
  let point = Point { x: 42, y: 0 };
  drop(point);
  point.x
}
```

An immutable `static` of a `Copy` type can be read by value. `drop(STATIC)` is
also a no-op for that type. A non-`Copy` static cannot be moved out (`E3058`).
Reading a `static mut` by value still requires `unsafe`:

```seal
static ORIGIN: Point = Point { x: 5, y: 0 };
static mut MUTABLE: Point = Point { x: 1, y: 2 };

func main() -> i32 {
  let origin = ORIGIN;
  let mutable = unsafe { MUTABLE };
  origin.x + mutable.y
}
```

A `const` always produces a fresh value and does not need `Copy`; an aggregate
that is `const` may be copied by value when its type implements `Copy`.

### References and loans

Copying an aggregate copies its reference fields and their checked loans. While
the copy is live, writing to the pointee conflicts with that loan:

```seal
struct Holder {
  value: *i32,
}

impl Copy for Holder {}

func main() -> i32 {
  let mut number = 1;
  let holder = Holder { value: &number };
  let copy = holder;
  // number = 2; // would conflict with the loan carried by `copy`
  *copy.value
}
```

A raw pointer field is different: copying it copies the address and its
provenance, but creates no checked loan. Dereferencing the copied raw pointer
still requires `unsafe`. A `Copy` aggregate never duplicates a pointee or an
allocation.

### Equality

`==` and `!=` read both operands without moving them, independently of `Copy`.
For a `Copy` struct or enum, the operands remain usable after the comparison.

## 7. Diagnostics

| Code | Meaning |
|------|---------|
| `E3076` | The implementation is specialized, omits/adds/reorders a parameter, or is otherwise not the one implementation for the whole declaration. Exact duplicates use coherence codes `E2021` or `E2058`. |
| `E3077` | A field, or a field of a variant, is not `Copy`; the message names the field/variant and its type, and a type parameter gets the `T: Copy` help. |
| `E3068` | The type also implements `Drop`; the `Copy` implementation is removed and the destructor remains registered. |
| `E2015` | The target is not a nominal struct or enum, or cannot be resolved as one. |
| `E2023` | The `impl Copy` block declares a function or method. |
| `E2066` | The program tries to declare a user trait named `Copy`. |
| `E3078` | `Copy` is used as a trait object or as a supertrait. |
| `E2018` | `Copy::name` names an associated item; `Copy` has none. |
| `E2027` | `Copy` appears in a type position; it is a trait, not a type. |
| `E3043` | A value does not satisfy a `T: Copy` bound. |
| `E3058` | A non-`Copy` value is moved out of a static. |
| `E3016` | A struct or enum contains itself by value. |

`Copy` has no associated functions, methods, or constants. It cannot be
redeclared, implemented as a supertrait, used as a trait object, or called as
`Copy::something`.

## 8. Complete example

[`examples/044_copy_optin.sl`](../../examples/044_copy_optin.sl) is the
acceptance example for this chapter. It declares a `Copy` struct, enum,
conditional `Wrapper<T>`, reference holder, and raw pointer holder, then
exercises assignment, an argument, a return, a field, a match, a by-value
`for`, a repeated array, indexing, `drop`, and a static read. It returns `42`,
and its assembly contains no drop glue for the `Copy` types.

The central shape is:

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

struct Holder {
  value: *i32,
}

impl Copy for Holder {}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  let copy = point;
  let number = 1;
  let holder = Holder { value: &number };
  let holder_copy = holder;

  drop(point);
  copy.x + point.y + *holder_copy.value
}
```

## 9. Quick reference

```text
Implementation
  impl Copy for Point {}
  impl<T: Copy> Copy for Wrapper<T> {}
  impl<T> Copy for Wrapper<T> where T: Copy {}

Accepted fields
  Copy scalars, references, raw pointers, arrays, tuples
  struct/enum with a validated Copy implementation

Rejected
  non-Copy field                       E3077
  type parameter without Copy bound    E3077
  specialized or wrong-arity impl      E3076
  Copy + Drop                          E3068
  function inside impl Copy            E2023
  non-nominal target                   E2015
  Copy as a trait object/supertrait    E3078
  Copy::name                           E2018
  Copy in a type position              E2027

Behavior
  assignment, argument, return         copies
  field or element read                copies
  match/if let/while let binding       copies
  for item in Copy array               copies; source usable
  drop(Copy value)                     no-op; value usable
  static of Copy type                  readable by value
  Copy + reference field               loan copied with the value
  Copy + raw pointer field             address copied; no checked loan
```
