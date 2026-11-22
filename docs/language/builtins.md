# Control-flow and introspection built-ins

Status: implemented in the current language version.

Seal provides eight built-in expressions beyond `len`, `drop`, and `assert`:

| Built-in | Result | Availability |
|----------|--------|--------------|
| `panic(message)` | `!` | every build |
| `exit(code)` | `!` | every build |
| `assert_eq(left, right[, message])` | `()` | `cfg(test)` and `cfg(bench)` |
| `assert_ne(left, right[, message])` | `()` | `cfg(test)` and `cfg(bench)` |
| `type_name::<T>()` | `*str` | every build |
| `type_of(value)` | `*str` | every build |
| `size_of::<T>()` | `usize` | every build |
| `align_of::<T>()` | `usize` | every build |

All of them are expressions, not functions. A user function or a local binding
with the same name shadows the built-in, exactly like `len`, `drop`, and
`assert`. Calls use ordinary argument syntax; the type-taking forms use the
ordinary turbofish syntax.

## 1. `panic(message)`

`panic` takes one `*str` value, reports it to standard error, and terminates the
process with status `101`. It never returns, so its type is `!` and it fits any
expression position, including the tail of a function with a non-unit result:

```seal
func load() -> i32 {
  let reason: *str = "configuration is missing";
  panic(reason)
}
```

A hosted target writes one line:

```text
panic: configuration is missing at src/lib.sl:3:3
```

The location is the span of the call. Panic never unwinds, never allocates, and
never runs implicit cleanup. Run destructors with `drop` before panicking when
their effects matter. A `runtime = "none"` target has no reporting runtime, so
the call lowers to a trap instead of writing a message; the process still does
not continue.

The message is not required to be a literal, so a computed `*str` is accepted:

```seal
let code = 7;
let reason = if code > 0 { "positive" } else { "non-positive" };
panic(reason);
```

A non-`*str` operand is `E3001`. The wrong number of arguments is `E3008`, and
explicit type arguments on this non-generic built-in are `E2053`.

## 2. `exit(code)`

`exit` takes one `i32` status and terminates the process immediately:

```seal
func stop() -> i32 {
  let status = 7;
  exit(status)
}
```

It does not return and has type `!`. Like `panic`, it performs no implicit
cleanup: call `drop(value)` first if a destructor must run. On a hosted target
the runtime uses the process-exit primitive directly; a freestanding target
calls the `exit` implementation supplied by the freestanding link. The status is
the exact `i32` value supplied, although POSIX shells expose only its low eight
bits.

Any integer type that is not `i32` is `E3001`; convert explicitly when needed.
The wrong number of arguments is `E3008`.

## 3. `assert_eq` and `assert_ne`

The equality assertions are available only in test and benchmark builds, like
`assert`. Using either in a normal build is `E8105`.

```seal
test func arithmetic_works() {
  assert_eq(2 + 2, 4);
  assert_ne(2 + 2, 5, "addition changed");
}
```

Both operands are checked with the ordinary equality rules and must have the
same supported type. The comparison reads its operands; it does not move them,
so both owners remain responsible for dropping their values. On success the
call produces `()`. On failure it reaches the ordinary panic path with the call
site and the optional message:

```text
FAIL arithmetic_works
     panic: assertion `left == right` failed at src/lib.sl:2:3
```

The default message names the failing operator. The optional third argument
must be a string literal; a non-literal message is `E8106`. Two or three
arguments are required, and any other count is `E3008`.

`assert_eq` is exactly `left == right` and `assert_ne` is exactly `left !=
right`; see the equality section in [`types.md`](../types.md) for the supported
types. There is no debug formatting of the operands, so a failed assertion
reports the message and source location without allocating.

## 4. Type introspection

The four introspection built-ins answer static questions at compile time. They
do not allocate. `type_name` and `type_of` materialize an interned string
constant; `size_of` and `align_of` produce an integer constant.

### `type_name::<T>()`

```seal
let integer = type_name::<i32>();          // "i32"
let array = type_name::<[u8; 4]>();        // "[u8; 4]"
let pointer = type_name::<*mut bool>();    // "*mut bool"
let named = type_name::<Pair<i32, u8>>();  // "Pair<i32, u8>"
```

The name uses the same source spelling as compiler diagnostics. A type
parameter is substituted before the name is produced, so a generic function
reports the concrete type of each instantiation:

```seal
func label<T>() -> *str {
  type_name::<T>()
}

let name = label::<u64>(); // "u64"
```

The type argument is required: omitting it is `E2052`. Value arguments are
rejected with `E3008`. Unsized types such as `str`, `[T]`, and `dyn Trait` are
accepted because only their name is needed.

### `type_of(value)`

```seal
let value = 42;
let name = type_of(value); // "i32"
```

`type_of` reports the static type the compiler assigned to its operand. The
operand is checked and evaluated for its effects but is read without being
moved, like an equality operand. A non-`Copy` value therefore remains usable:

```seal
let token = Token { tag: 7 };
let name = type_of(token); // "Token"
drop(token);                // still valid
```

The operand's type is exactly the type the compiler assigned to the value
expression. `type_of` takes no type arguments and exactly one value argument; a
type argument is `E2053`, and a wrong value count is `E3008`.

### `size_of::<T>()` and `align_of::<T>()`

```seal
let size = size_of::<u64>();     // 8
let align = align_of::<Mixed>(); // 8
```

Both return `usize` and use the target layout, including the target pointer
width for `isize`, `usize`, and references. They follow the same layout rules
as storage, so `size_of` and `align_of` agree with field offsets and allocation
sizes.

The queried type must be sized. `size_of::<str>()`, `align_of::<[T]>()`, and
`align_of::<dyn Trait>()` are rejected with `E3010`; a type whose array size
exceeds the target address space is `E3023`. A concrete aggregate is valid even
when it is generic, and a type parameter is substituted per instantiation.

### Compile-time evaluation

All four queries are constants, so they may appear in a `const` or `static`
initializer:

```seal
const INTEGER_NAME: *str = type_name::<i32>();
const POINTER_SIZE: usize = size_of::<*mut u8>();

test func layout_is_stable() {
  assert_eq(POINTER_SIZE, size_of::<usize>());
  assert_eq(INTEGER_NAME, "i32");
}
```

`panic` and `exit` cannot complete constant evaluation. A compile-time path that
reaches either reports `E4027`, because a constant must produce a value without
ending the compiler process.
