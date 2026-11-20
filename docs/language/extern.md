# External functions and statics

Status: declared functions, calls, static storage, native library resolution,
C-layout structs passed and returned by value, opaque types, `c_void`, variadic
declarations, C function pointer types, callbacks, and indirect calls are
implemented in the current language version.

An `extern` declaration names code or storage that C defines and the linker
resolves. Seal does not own the body or the storage: it emits a declaration,
checks the signature, and requires every use to appear inside an `unsafe`
block. The values crossing the boundary still obey Seal's ordinary type rules.

```seal
extern "c" {
  func abs(value: i32) -> i32;
  static mut errno: *raw mut c_int;
}

func main() -> i32 {
  unsafe { abs(-41) }
}
```

The program calls `abs` from the C runtime and returns `41`. No link flag is
needed for a symbol the platform's C library already exports.

## 1. Declaration syntax

A declaration is either a block or a single item.

```text
extern "ABI" {
  link "library";
  func name(parameters[, ...]) -> Return link_name "symbol";
  static [mut] name: Type;
}
extern "ABI" func name(parameters[, ...]) -> Return link_name "symbol";
extern "ABI" static [mut] name: Type;
extern struct Name { field: Type, ... }
extern type Name;
```

- The ABI is always a string. `"c"`, `"cdecl"`, and `"system"` are accepted and
  all mean the target's default C convention. Any other spelling, including
  `"stdcall"` and `"fastcall"`, is `E3080` because the compiler cannot emit it.
- `func` declarations have names, typed parameters, and an optional return
  type. There is no receiver, body, generic parameter list, `where` clause, or
  `unsafe` marker: an external function is implicitly unsafe to call.
- A trailing `...` declares a C variadic function. It must follow at least one
  named parameter, may appear once, and must be the last element of the list.
  `func f(...);`, `func f(x: c_int, ..., y: c_int);`, and a Seal `func` with
  `...` are `E3088`. A variadic declaration may be called with zero extra
  arguments, which is how C's `printf`-style contract is expressed.
- `static` declarations have a name and a type. `static mut` is the only form
  Seal may write.
- A member of a block inherits the block's ABI. A member cannot itself be an
  `extern` block (`E3092`).
- `link "name";` inside a block asks the linker for a native library. The name
  is a library name, not a path: pass the directory separately with `-L` and
  the library with `-l` on the command line. Link directives may appear in any
  order among the declarations and are resolved in source order.
- `extern` is contextual. Outside item position it is an ordinary identifier,
  and a function or binding may be named `extern`.

### C structs and opaque types

An `extern struct` is a struct with the C layout. It is a normal Seal type
otherwise: it has fields, literals, methods, patterns, `Copy` opt-in, and it may
implement `Drop`. The declaration has no type parameters and no `where` clause;
writing either is `E3093`.

```seal
extern struct Point { x: c_int, y: c_int }

extern "c" {
  func ffi_point_new(x: c_int, y: c_int) -> Point;
  func ffi_point_sum(point: Point) -> c_int;
}
```

Every field must have a non-zero C representation. `()` and another zero-sized
field are `E3085`, `bool` is `E3085` with the suggestion to use `c_bool`, and a
non-FFI-safe field such as `*T`, `str`, or a Seal-layout struct is `E3084`. A
nested `extern struct` and an array of C-safe elements are accepted. This is
what makes the layout of the whole aggregate provable against the C compiler.

An `extern type` names storage whose size and layout only C knows:

```seal
extern type FILE;

extern "c" {
  func fopen(path: *raw u8, mode: *raw u8) -> *raw mut FILE;
  func fclose(file: *raw mut FILE) -> c_int;
}
```

An opaque type is `!Sized`, has no fields, no layout, and no destructor. It may
only appear as the pointee of a raw pointer. Writing it by value in a binding, a
field, an array, a parameter, or a return type is `E3087`; dereferencing a raw
pointer to it is also `E3087`. The built-in `c_void` follows the same rule and is
the intended pointee for an untyped C pointer.

### C function pointer types

A C function pointer is written as a raw pointer to a C function signature:

```seal
*raw extern "c" func(i32, *raw c_void) -> c_int
```

The ABI string and `func` are part of the type. Parameters are types only; a
function pointer type does not name its parameters. A trailing `...` marks a
variadic signature and must follow at least one fixed parameter. Omitting the
result means C `void`.

The signature is unsized and only appears as the pointee of `*raw`. The pointer
itself is one word, is `Copy`, and is FFI-safe. Every fixed parameter and the
result pass through the same FFI-safety predicate as an external declaration,
so `char`, checked references, Seal-layout aggregates, tuples, and opaque values
by value are `E3084`. The signature is read-only: `*raw mut extern func` is
`E3089`.

A Seal function becomes a C function pointer with an explicit cast:

```seal
extern "c" { func apply(callback: *raw extern "c" func(i32) -> i32, value: i32) -> i32; }

func double_it(value: i32) -> i32 { value * 2 }

func main() -> i32 {
  unsafe {
    let callback = double_it as *raw extern "c" func(i32) -> i32;
    apply(callback, 21)
  }
}
```

The function must have no receiver, its instantiated signature must match the
target exactly (including variadicity), and a generic function needs explicit
type arguments: `identity::<i32> as ...`. Taking the address and converting an
arbitrary raw pointer to a function pointer both require `unsafe`. Calling
through the pointer also requires `unsafe`, because the compiler cannot prove
that the address is non-null and callable with the declared signature.

A Seal function is emitted with Seal's internal ABI. An aggregate parameter or
result would therefore disagree with a C caller, so the compiler emits one
cached C-ABI trampoline per `(function instance, pointer type)`. The trampoline
receives C arguments, converts them, calls the internal function, and converts
the result back. The internal function is not marked `noalias` when its address
escapes, because C may call it through a path the whole-program aliasing proof
did not model.

### Return types and `void`

Omitting the return type and writing `-> ()` both declare a C `void` function.
Calling it produces a unit value, and code generation emits a call without a
return value. Writing any other type returns that value.

```seal
extern "c" {
  func free(pointer: *raw mut u8) -> ();
  func getpid() -> i32;
}
```

### Linking a library

A library is named either in the source or on the command line:

```seal
extern "c" {
  link "sealfixture";
  func ffi_scalar_i32(value: i32) -> i32;
}
```

```text
sealc build -L /path/to/libs -l sealfixture program.sl
```

`-l NAME` and `-L DIR` are repeatable, and `--link-arg ARG` passes one argument
straight to the linker (repeat it for several arguments; values beginning with
`-` are accepted). Libraries named in the source come first, then `-l`
libraries, then `--link-arg` values, and the Seal runtime archive is last. The
linker program is selected by `SEAL_LINKER`, then `CC`, then the platform
default.

A library name may not be empty or contain whitespace, control characters, or a
path separator; every `-L` must be an existing directory. These rules are
checked before any temporary file is written and reported as `E6005`. Object
and assembly output do not link, so they ignore libraries and search paths.

### Linker symbols and `link_name`

The linker symbol is the declared name unless `link_name "..."` overrides it.
The Seal name stays independent: the program calls `absolute`, while the object
file asks the linker for `abs`.

```seal
extern "c" func absolute(value: i32) -> i32 link_name "abs";
```

A symbol must be non-empty, contain no NUL byte, whitespace, or control
character, and must not be `main` (`E3082`); `main` belongs to the Seal entry
point. Two declarations of one symbol are accepted only when their ABI and
signature or storage type agree. A disagreement is `E3083`, reported once with
both declaration sites.

Declaring a Seal name twice is an ordinary duplicate definition: `E2001` for a
function and `E2068` for a static. The symbol check and the Seal-name check are
independent, so two names may intentionally share one C symbol.

## 2. Calling an external function

An external call has the shape of any other call and is type-checked the same
way: arity, parameter types, and the declared return type all participate in
unification. The only extra rule is the unsafe gate.

```seal
extern "c" { func getpid() -> i32; }

func main() -> i32 {
  let pid = getpid(); // E3070
  let pid = unsafe { getpid() };
  if pid <= 0 { return 1; }
  42
}
```

`E3070` is the existing unsafe-operation diagnostic. Its note names the
external function, so the fix is visible without reading the declaration.

The unsafe block is an obligation, not a permission: the programmer promises
that the symbol exists in the final link, that the declared signature matches
the C definition, and that every argument satisfies the C function's contract.
The compiler cannot verify any of those.

A variadic declaration checks the fixed parameters exactly as usual and accepts
any number of extra arguments. C applies its default argument promotions to
them, and the compiler makes each promotion explicit:

- `bool`, `i8`, `u8`, `i16`, and `u16` become `c_int` (`i32`). `bool` becomes
  `0` or `1`.
- `f32` becomes `f64`.
- `i32`/`u32`, `i64`/`u64`, `isize`/`usize`, `f64`, and thin raw pointers keep
  their type.
- An aggregate, enum, `char`, checked reference, fat pointer, slice, string,
  or 128-bit integer is `E3088` at the argument. Passing a pointer to an
  aggregate is fine; the pointer itself is thin.

```seal
extern "c" { func snprintf(buffer: *raw mut u8, size: usize, format: *raw u8, ...) -> c_int; }

func main() -> i32 {
  let mut buffer: [u8; 16] = [0u8; 16];
  let written = unsafe {
    snprintf(&mut buffer[0] as *raw mut u8, 16usize, c"value=%d", 7)
  };
  if written != 7 { return 1; }
  42
}
```

## 3. External statics

An external static names storage that C allocates. Reading it requires
`unsafe`; writing it additionally requires the declaration to be `static mut`.
Both violations are `E3086`.

```seal
extern "c" { static mut optind: c_int; }

func next_index() -> c_int {
  unsafe {
    optind = optind + 1;
    optind
  }
}
```

The declared mutability describes what Seal is allowed to do, not what C does.
C may change a `static` that Seal declared immutable at any time, which is why
even a read is unsafe and why the compiler treats an external static access as
an opaque effect. An immutable declaration that is written is rejected; a
mutable one may be read and written.

Addressing an external static, including taking a raw pointer to it, happens
through the same `unsafe` gate. The linker resolves the symbol exactly as it
resolves an external function.

## 4. Types at the boundary

Parameters and return types must be FFI-safe; the rules and the exact reasons
are in [`../types.md`](../types.md). In particular:

- C scalar names (`c_int`, `c_long`, `size_t`, and the rest) are the intended
  spelling for fixed C widths.
- `bool`, the fixed-width integers, `f32`, `f64`, and thin raw pointers are
  FFI-safe.
- Checked references (`*T`, `*mut T`), `char`, `i128`/`u128`, fat raw pointers,
  slices, strings, trait objects, tuples, payload enums, and structs without
  C layout are rejected with `E3084`.
- An `extern struct` is FFI-safe by value when every field is; its layout is
  the target's C layout and its fields keep declaration order.
- An `extern type` (and `c_void`) is only FFI-safe behind `*raw`; by value it is
  `E3087`.
- An external static may have any declared type that a Seal static may have;
  accessing it safely is the caller's responsibility.
- A C string literal `c"..."` has type `*raw u8` and points at NUL-terminated
  static read-only bytes, so it can be passed directly to a `const char *`
  parameter. It is a thin raw pointer and therefore FFI-safe.

Every offending parameter, return type, or static type is reported at its own
span, with the reason and a suggestion. One declaration can therefore produce
several `E3084` diagnostics when several types are wrong.

## 5. Constant evaluation

A constant cannot cross the FFI boundary. Calling an external function while
evaluating a constant is `E4033`; reading an external static is `E4034`.

```seal
extern "c" { func abs(value: i32) -> i32; }
const VALUE: i32 = unsafe { abs(-1) }; // E4033
```

The unsafe block does not make the operation constant. C code runs at program
start, not during compilation, and an external static has no value the compiler
can inspect.

A `c"..."` literal is not a call: it evaluates to a static address and may be
used to initialize a `const` or `static` of type `*raw u8`. The bytes end in a
NUL that is not part of the written contents, so C sees the terminator while
Seal's string length, when a `*str` is used instead, excludes it.

## 6. Code generation and linking

Each declaration produces exactly one LLVM declaration, cached by its HIR
definition:

- Functions use `external` linkage, the target's C calling convention, and no
  attributes beyond the ABI-mandated `byval` and `sret` on aggregate positions.
  A variadic declaration ends its LLVM function type with `...`, and every
  extra argument is passed at its promoted type. The compiler does not own the
  body, so it cannot promise `noalias`, `readonly`, `nounwind`, or any other
  property.
- A `c"..."` literal becomes one NUL-terminated private string constant. The
  same constant cache serves `*str` and `c"..."`; a `*str` carries its length
  separately and ignores the terminator.
- A C-layout struct is coerced exactly as C coerces it. On System V AMD64,
  structs up to 16 bytes become integer or SSE pieces in registers (for
  example `{i32, i32}` becomes one `i64`), while larger structs are returned
  through a hidden `sret` pointer and passed with `byval`. On Microsoft x64,
  structs of 1, 2, 4, or 8 bytes become one integer and every other size is
  passed and returned indirectly. The declaration, every call, and the result
  conversion use one cached classification, so the two sides cannot disagree.
- Statics use `external` linkage and are never marked `constant`, because C may
  initialize them at load time. A `static mut` declaration produces writable
  storage; an immutable one is still emitted as a mutable global.
- A `void` external call does not write a destination.
- Debug builds describe the declaration with a subprogram that has no body.

The program fails at link time, not at compile time, when the symbol is missing
from every linked library. Native libraries named by `link` directives or by
`-l`, their search paths from `-L`, and raw linker arguments from `--link-arg`
all reach the linker in a deterministic order. The C runtime's own symbols are
available without any flag on the supported targets.

A failed link is `E6003`; its message includes the linker status, the full
argument vector, and the libraries that were requested. Invalid configuration
is `E6005` and is rejected before the linker runs, so no temporary object,
runtime archive, or partial executable is left behind.

## 7. Diagnostics

| Code | Meaning |
|------|---------|
| `E2001` | Two external functions declare the same Seal name |
| `E2068` | Two values declare the same Seal name |
| `E3070` | An external call appears outside `unsafe` |
| `E3080` | The ABI spelling is unknown or unavailable |
| `E3081` | The declaration is malformed, or uses an unsupported member |
| `E3082` | The linker symbol is empty, reserved, or contains invalid characters |
| `E3083` | One symbol is declared with two different shapes |
| `E3084` | A parameter, return type, field, or static type is not FFI-safe |
| `E3085` | An external struct is empty, or a field has no valid C representation |
| `E3086` | An external static is accessed outside `unsafe`, or an immutable one is written |
| `E3087` | An opaque type is used by value or dereferenced |
| `E3088` | A variadic declaration or argument is malformed, or its type cannot cross `...` |
| `E3089` | A C function pointer is mutable, mismatched, generic without instantiation, has a receiver, or is used outside `unsafe` |
| `E3090` | A `c"..."` literal contains an interior NUL |
| `E3091` | A `link` directive is malformed (missing string, extra token, or missing `;`) |
| `E3092` | An `extern` declaration is nested inside another `extern` block |
| `E3093` | An external function, struct, or opaque type is generic |
| `E4033` | A constant calls an external function or C function pointer, or takes a C function address |
| `E4034` | A constant reads an external static |
| `E6003` | The linker failed; the message names the command and libraries |
| `E6005` | A library name or `-L` search path is invalid |

## 8. Current limits

The following are deliberately rejected, never silently miscompiled:

- Seal cannot define its own variadic function; only an `extern` declaration
  may end in `...` (`E3088`).
- A Seal function pointer is not a first-class value type: only the raw C form
  exists, and taking a Seal function's address is an explicit cast.
- Per-declaration attributes such as `noalias` or `noreturn` are not accepted
  on an external declaration.
- C aggregates by value on an architecture without an implemented aggregate
  ABI are rejected with `E6002`; x86_64 System V and Microsoft x64 are
  implemented.

## 9. Complete example

```seal
extern "c" {
  func abs(value: i32) -> i32;
  func getpid() -> i32;
  func srand(seed: u32) -> ();
  func snprintf(buffer: *raw mut u8, size: usize, format: *raw u8, ...) -> c_int;
}

func main() -> i32 {
  unsafe {
    srand(7u32);
    if abs(-41) != 41 { return 1; }
    if getpid() <= 0 { return 2; }
    let mut buffer: [u8; 16] = [0u8; 16];
    let written = snprintf(&mut buffer[0] as *raw mut u8, 16usize, c"value=%d", 7);
    if written != 7 { return 3; }
    if buffer[7] != 0 { return 4; }
  }
  42
}
```

A correct run exits with `42`. The example is exercised at `-O0` with debug
information and at `-O2` without it.
