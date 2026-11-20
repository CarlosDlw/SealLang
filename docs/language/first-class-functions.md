# First-class functions and closures

Seal has three related callable values: a **function pointer** written
`func(A, B) -> R`, a **closure** written `|x| ...`, and a **generic callable**
constrained by `Fn`, `FnMut`, or `FnOnce`. All three are ordinary `Sized`
values with value semantics. A function pointer is one word, a closure is an
inline aggregate with one field per capture, and calling either one costs the
same as an ordinary call except for the indirect call through a pointer.

The type system and the borrow checker see the same values as the rest of the
language. There is no hidden heap allocation, no boxing, no reference counting,
and no runtime type search. A closure that escapes the frame must be converted
to a `func` pointer, and a conversion is allowed only when the closure captures
nothing.

## 1. Function pointer types

```seal
func add(a: i32, b: i32) -> i32 { a + b }
func notify() {}

func main() -> i32 {
  let sum: func(i32, i32) -> i32 = add;
  let callback: func() = notify;
  sum(20, 22)
}
```

`func(...)` is the internal Seal calling convention. The result may be omitted,
in which case it is `()`. Parameters are always fixed; a Seal function type is
never variadic. A C signature is a different type and is written only behind a
raw pointer: `*raw extern "c" func(...)`. The two never convert implicitly.

Function pointer types have these properties:

| Property | Value |
|----------|-------|
| `Sized` | yes |
| `Copy` | yes |
| Size and alignment | one pointer |
| Destructor | none |
| Null value | no; there is no null literal or integer conversion |
| Equality | no (`==` and `!=` are rejected) |
| Arithmetic or casts | no; not to or from an integer or raw pointer |
| Calling convention | internal Seal ABI |
| Variadic | no |

`unsafe` is part of the type. Calling an `unsafe func` value requires an
`unsafe { ... }` block, exactly like calling an `unsafe func` declaration:

```seal
unsafe func unchecked(value: i32) -> i32 { value + 1 }

func main() -> i32 {
  let callback: unsafe func(i32) -> i32 = unchecked;
  unsafe { callback(41) }
}
```

A safe function converts to a safe pointer or to an unsafe pointer with the same
signature. An unsafe function converts only to an unsafe pointer: dropping the
safety marker would silently remove the caller's contract (`E3098`). A safe
pointer does not satisfy a `Fn` bound if the bound would hide an unsafe call;
`unsafe func` values do not implement the function traits.

Function pointers can be stored, passed, returned, and placed in aggregates:

```seal
struct Ops {
  run: func(i32) -> i32,
}

func add_one(value: i32) -> i32 { value + 1 }
func double(value: i32) -> i32 { value * 2 }

func choose(flag: bool) -> func(i32) -> i32 {
  if flag { add_one } else { double }
}

func main() -> i32 {
  let selected = choose(true);
  let ops = Ops { run: selected };
  let pair = (ops.run, 20);
  let array = [double, add_one];
  (pair.0)(pair.1) + array[1](0)
}
```

The expression returns `22`. Layout is recursive: a struct or array containing
a `func` stores one pointer per element and introduces no allocation.

### Function items as values

A free function name is a value with the function's concrete pointer type. The
compiler accepts it when the expected type is `func(...)`; otherwise use an
explicit `as` cast:

```seal
func add_one(value: i32) -> i32 { value + 1 }

func main() -> i32 {
  let inferred: func(i32) -> i32 = add_one;
  let explicit = add_one as func(i32) -> i32;
  inferred(20) + explicit(20)
}
```

A generic function needs its arguments from the expected `func` type or an
explicit turbofish. A bare generic function in a value position with no expected
signature is rejected (`E3099`):

```seal
func identity<T>(value: T) -> T { value }

func main() -> i32 {
  let first: func(i32) -> i32 = identity;
  let second = identity::<i32>;
  first(1) + second(41)
}
```

A method with a receiver, a trait function without a concrete implementation,
and an `extern "c"` function are not Seal function values (`E3099`). Wrap the
operation in a closure, or use the existing C function pointer spelling for a C
callback.

## 2. Closure syntax

```text
closure        := `move`? `|` closure_params? `|` closure_body
closure_params := closure_param { `,` closure_param } `,`?
closure_param  := IDENT [ `:` Type ]
closure_body   := Expr | `->` Type Block
```

Examples:

```seal
let unit = || 1;
let one = |value: i32| value + 1;
let two = |left, right| left + right;
let trailing = |left, right,| left + right;
let explicit = |value: i32| -> i32 { value + 1 };
let moved = move |value: i32| value + 1;
```

`||` is the closure with no parameters. In prefix position `|` starts a
closure; after an operand it remains the bitwise-or operator, so
`left | right` is unchanged. A body after `-> Type` must be a block, because
`|x| -> i32 x` would otherwise be ambiguous with the `|` operator.

Closure parameters are names or `_`, optionally annotated. Patterns, `self`,
attributes, and closure generics are not part of this surface. When an
annotation is missing, the expected type supplies it. A closure used where no
signature can be inferred is `E3094`:

```seal
let bad = |value| value + 1; // E3094: no expected signature
```

## 3. Capture

A closure stores the values it uses from the enclosing frame in a synthetic
environment. The environment is an inline aggregate with one field per captured
variable, in first-mention order. Capturing is by whole variable, not by field:
using `point.x` captures `point`.

Without `move`, a capture is stored as a reference. A read-only use stores
`*T`; a write or an exclusive borrow stores `*mut T`. The closure sees the
original storage, and the borrow checker keeps the loan live for as long as the
closure is used:

```seal
func main() -> i32 {
  let value = 40;
  let add = || value + 2;
  add()
}
```

`move` captures by value. A `Copy` value is copied, and a non-`Copy` value is
moved into the environment. The capture is evaluated when the closure is
created, so a moved `Copy` value is a snapshot:

```seal
func main() -> i32 {
  let mut value = 10;
  let read = move || value;
  value = 32;
  read() // 10
}
```

Without `move`, the same assignment is rejected while the closure is live:

```seal
func main() -> i32 {
  let mut value = 10;
  let read = || value;
  value = 32; // E5001: `value` is borrowed
  read()
}
```

That is the same loan rule as any other reference. To mutate state through a
closure, capture an exclusive borrow; the closure then has kind `FnMut`:

```seal
func main() -> i32 {
  let mut total = 0;
  let mut add = |value: i32| { total += value; };
  add(2);
  add(3);
  total // 5
}
```

A non-`move` closure cannot move a non-`Copy` capture out of the frame, because
the frame still owns it (`E3095`). Add `move` to transfer ownership:

```seal
struct Token { value: i32 }

func main() -> i32 {
  let token = Token { value: 42 };
  let take = move || token;
  let consumed = take();
  consumed.value
}
```

A nested closure captures through the immediately enclosing environment. Shared
captures are copied as references, so a child closure can outlive the parent's
use of the field without creating a conflicting loan:

```seal
func main() -> i32 {
  let value = 40;
  let outer = || {
    let inner = || value + 2;
    inner()
  };
  outer()
}
```

Statics and constants are not captures. A `static` is already a global place,
and a `const` is a value substituted at the use.

The captured environment follows the ordinary drop rules. Fields drop in
capture order, and a value moved out of an `FnOnce` environment is not dropped
again by the environment; drop elaboration tracks initialization per field.

## 4. Callable kinds

Every closure implements one of three compiler traits, chosen by how its body
uses the captures:

| Kind | Use in the body | Receiver at the call |
|------|-----------------|----------------------|
| `Fn` | reads or shares every capture | `*Self` |
| `FnMut` | writes at least one capture, consumes none | `*mut Self` |
| `FnOnce` | moves at least one capture out | `Self` |

The capture mode does not choose the kind. `move || value + 1` with a `Copy`
value is still `Fn`, because it only reads the owned capture. `move` matters when
a non-`Copy` value is moved out, which is what makes a closure `FnOnce`.

The hierarchy is cumulative:

```text
Fn: FnMut: FnOnce
```

A value that implements `Fn` can be used where `FnMut` or `FnOnce` is expected;
`FnMut` can be used where `FnOnce` is expected; `FnOnce` cannot be used where a
weaker bound is requested. Calling a specific closure uses the strongest kind
the closure implements:

```seal
func twice<F: Fn() -> i32>(f: F) -> i32 { f() + f() }

func each<F: FnMut(i32)>(f: F, n: i32) {
  let mut f = f;
  let mut index = 0;
  while index < n {
    f(index);
    index += 1;
  }
}

func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() }
```

`each` needs a mutable binding because it calls `f` through `*mut Self`.
`consume` consumes `f`, so an `FnOnce` value cannot be called twice. A `Copy`
closure can be called twice through an `FnOnce` bound because the copy is
consumed. A concrete `FnOnce` closure cannot be called twice (`E5005`).

The closure type is `Copy` exactly when every environment field is: shared
captures are references and are `Copy`; value captures are `Copy` when the
captured value is; an exclusive capture makes the closure non-`Copy`, even when
the raw pointer representation would be copyable. This prevents two closures
from duplicating one exclusive loan.

## 5. Calls and bounds

A call `f(args)` selects the callable path from the callee type:

- a concrete closure calls its lifted body with the environment;
- a `func` value performs an indirect call with the internal Seal ABI;
- a generic `F: Fn(...)` stays symbolic until monomorphization selects the
  concrete closure or pointer;
- the explicit method `f.call(...)` is rejected (`E3096`). The call operator is
  the public surface, and the trait methods exist for the type system.

Bounds use the sugar `Fn(A, B) -> R`, `FnMut(A)`, or `FnOnce() -> R`. The
explicit form `Fn<(A, B), R>` is equivalent; `Args` is always a tuple, including
`()`:

```seal
func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 {
  f(value)
}

func main() -> i32 {
  let base = 40;
  apply(|value| value + base, 2)
}
```

A bound may appear inline or in a `where` clause. Two bounds on one parameter
must agree on the callable signature; disagreement is `E3096`. An `Fn` bound
rejects an `FnMut`/`FnOnce` closure (`E3096`), and an unsafe function pointer
does not satisfy any function-trait bound.

`dyn Fn`, `impl Fn`, and declaring or implementing `Fn`/`FnMut`/`FnOnce` are not
part of this surface. `dyn Fn(...)` is `E3096`; a user declaration or
implementation is `E3097`.

## 6. Coercions

A function item converts to a `func` type with the identical parameter list,
result, and safety. A capture-free closure converts to a matching `func` type,
and the compiler emits an internal trampoline that supplies the empty
environment:

```seal
func main() -> i32 {
  let add: func(i32) -> i32 = |value| value + 1;
  add(41)
}
```

A closure with captures cannot become a function pointer, because the pointer
has no place to store the environment (`E3098`). Use a generic `F: Fn(...)`
bound, or make the state explicit in a parameter:

```seal
func make_adder(delta: i32) -> func(i32) -> i32 {
  |value| value + delta // E3098: captures `delta`
}
```

`move` does not change this rule: a moved capture is still state that a bare
pointer cannot carry. A closure that captures only `Copy` values is still a
closure with captures.

`func` does not convert to or from `*raw extern "c" func` implicitly. Use the
explicit C callback cast when crossing the C boundary; the two calling
conventions classify aggregates differently.

## 7. Constants and compile-time evaluation

A function address or closure is not a compile-time value. `const F: func(...)
= foo;` is `E4033`, as is calling a closure or function pointer in a `const` or
`static` initializer. Constructing a closure and calling it at runtime is fine;
constant evaluation never guesses an indirect target.

## 8. Cost and ABI

- A function pointer is one word and a value call is one indirect call.
- A closure is an inline aggregate; creating one writes its fields, with no heap
  allocation and no reference counting.
- A closure body is a lifted function whose first parameter is the environment:
  `*Env` for `Fn`, `*mut Env` for `FnMut`, and `Env` by value for `FnOnce`.
- A `func` call and a closure call use the internal Seal ABI. Aggregates pass
  directly; no C `sret`/`byval` classification is involved.
- A capture-free closure converted to `func` uses one internal trampoline per
  concrete instance. No C trampoline is emitted for a Seal function value.
- A shared environment parameter is `readonly`; an exclusive one receives
  `noalias` only when the whole-program proof holds.

## 9. Diagnostics

| Code | Meaning |
|------|---------|
| `E3094` | Closure parameter or result cannot be inferred, or the closure signature conflicts with the expected signature |
| `E3095` | The body tries to move out of a capture stored by reference; add `move` |
| `E3096` | Unsupported callable use: malformed `Fn` bound, ambiguous bounds, `dyn Fn`, explicit `call`, or calling a non-callable |
| `E3097` | Declaring or implementing `Fn`, `FnMut`, or `FnOnce` |
| `E3098` | Incompatible conversion to `func`: captures, signature mismatch, or unsafe-to-safe |
| `E3099` | Invalid `func` type or a function that cannot become a value: variadic/ABI spelling, method with receiver, trait function, extern C, or generic without instantiation |
| `E5001` | Writing a place while a closure holds a shared loan |
| `E5005` | Using a moved `FnOnce` closure again |
| `E4033` | Function address or indirect call in a constant context |

## 10. Limits and future work

This batch deliberately excludes `dyn Fn`/`impl Fn`, explicit capture lists,
capture precision by field, patterns in closure parameters, `async`/`const`
closures, generators, closure boxing, equality/ordering/nullability/arithmetic
for `func`, implicit conversion between `func` and C function pointers, methods
as values, and overloading. Each exclusion has a stable diagnostic or a
documented path rather than a silent fallback.

The architecture leaves room for them: closure values already carry their kind
and environment, function pointers already have a structural interned type, and
the C callback trampoline is separate from the internal ABI. Adding `dyn Fn`
needs vtable thunks; field-precise captures need a projection list in the
capture definition; boxing needs an owning allocation, not a new callable ABI.

## 11. Complete example

```seal
func add_one(value: i32) -> i32 { value + 1 }

func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }

func twice<F: Fn() -> i32>(f: F) -> i32 { f() + f() }

func main() -> i32 {
  let base = 10;
  let by_ref = |value: i32| value + base;
  let by_value = move |value: i32| value + base;
  let mut total = 0;
  let mut add_to_total = |value: i32| { total += value; };
  add_to_total(3);
  apply(add_one, 1) + apply(by_ref, 10) + apply(by_value, 5) + twice(|| 1) + total
}
```

The result is `42`. The runnable versions are
[`examples/046_function_pointers.sl`](../../examples/046_function_pointers.sl),
[`examples/047_closures.sl`](../../examples/047_closures.sl), and
[`examples/048_function_traits.sl`](../../examples/048_function_traits.sl).
