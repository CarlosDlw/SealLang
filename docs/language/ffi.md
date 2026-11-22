# C interoperability (FFI)

Status: implemented in the current language version. The surface includes
`extern "c"` functions, statics and link directives; C-layout structs by value;
opaque types and `c_void`; variadic declarations and promotions; `c"..."`
literals; C function pointer types, callbacks and indirect calls;
`with_len`/`as_str`; native library linking from source and the command line;
and `export "c"` functions that C can call from a static or shared library.

An `extern` declaration names code or storage that C defines and the linker
resolves. Seal does not own the body or the storage: it emits a declaration,
checks the signature, and requires every use to appear inside an `unsafe`
block. The values crossing the boundary still obey Seal's ordinary type rules.

This chapter is the complete guide to the feature. It covers the grammar, the
Seal ↔ C ↔ LLVM type table, ABI rules per target, aggregates, opaque handles,
variadics, C strings, function pointers and callbacks, statics, linking,
constant evaluation, the safety contract, diagnostics, and a complete runnable
example. The normative type and memory-model decisions live in
[`../types.md`](../types.md) and [`../memory-model.md`](../memory-model.md).

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

### C scalar names and the LLVM representation

The C scalar names are built-in type names. They resolve from the target's C
ABI, never from the host, so a cross-compiled object has the same meaning as a
native one. They are transparent aliases: `c_int` is `i32`, and a `c_int` value
is accepted wherever an `i32` is expected.

| Seal spelling | C type | LLVM type | Notes |
|---------------|--------|-----------|-------|
| `c_char` | `char` | `i8` or `u8` | Signedness follows the target |
| `c_schar` | `signed char` | `i8` | Always signed |
| `c_uchar` | `unsigned char` | `u8` | Always unsigned |
| `c_short` / `c_ushort` | `short` / `unsigned short` | `i16` | |
| `c_int` / `c_uint` | `int` / `unsigned int` | `i32` | |
| `c_long` / `c_ulong` | `long` / `unsigned long` | `i64` on LP64, `i32` on LLP64 and 32-bit targets | |
| `c_longlong` / `c_ulonglong` | `long long` / `unsigned long long` | `i64` | |
| `c_float` | `float` | `f32` | |
| `c_double` | `double` | `f64` | |
| `c_bool` | `_Bool` in an aggregate | `i8`, holding 0 or 1 | Field-only spelling |
| `bool` | `_Bool` as a direct parameter or result | `i1` | FFI-safe; see the aggregate rule below |
| `c_wchar` | `wchar_t` | `i16` on Windows, `i32` elsewhere | Numeric only, not a string type |
| `size_t` / `ssize_t` | `size_t` / `ssize_t` | pointer-sized unsigned / signed integer | |
| `ptrdiff_t` / `intptr_t` / `uintptr_t` | same | pointer-sized integer | |

`c_char` is signed on Windows, Darwin, and the x86 family, and unsigned on ARM,
AArch64, RISC-V, and WebAssembly. `c_long` is 64 bits on LP64 Unix and 32 bits on
Windows x64 and 32-bit targets. `wchar_t` is 16-bit unsigned on Windows and
32-bit signed elsewhere. The compiler exposes these facts through the target
specification; `sealc build --target <triple>` uses the target's values, not the
host's.

`c_bool` exists so an aggregate field has the one-byte C representation. A
direct `bool` parameter or result is also FFI-safe and uses LLVM `i1`, which is
the representation Clang and GCC use for `_Bool` in a function signature. A
`bool` field in an `extern struct` is rejected with `E3085`; write `c_bool`
there instead.

### ABI by target

Scalars use the target's C calling convention directly. Aggregates are not
passed as their raw LLVM struct type: the compiler classifies them with the
same rules as the target C compiler and coerces them at the declaration and at
every call site.

| Target family | Aggregate by value |
|---------------|--------------------|
| System V AMD64 (Linux, macOS, BSD) | Up to 16 bytes become integer/SSE pieces; larger values are passed `byval` and returned through hidden `sret` storage |
| Microsoft x64 | 1, 2, 4, or 8 bytes become one integer; every other size is passed and returned indirectly |
| Other architectures | Scalars compile; an aggregate by value is `E6002` because its C aggregate ABI is not implemented |

The classification walks nested fields and arrays, uses the target layout for
offsets and padding, and caches one answer per type and target. A struct is
never silently passed with the wrong shape: unsupported targets fail during
compilation.

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

### C buffers and fat pointers

A C API frequently returns a data pointer and a length in separate values.
`with_len` assembles them into a Seal raw slice pointer, and `as_str`
reinterprets a `*raw [u8]` as `*raw str`:

```seal
extern "c" {
  func ffi_fill_buffer(out: *raw mut u8, cap: usize) -> ();
}

func main() -> i32 {
  let mut buffer = [0u8; 8];
  unsafe {
    let base = &mut buffer[0] as *raw mut u8;
    ffi_fill_buffer(base, 8usize);
    let bytes: *raw mut [u8] = base.with_len(5);
    let text: *raw mut str = bytes.as_str();
    let first = (*bytes)[0];
    let length = text.len();
    let data = text.data();
  }
  0
}
```

Both methods are `unsafe`: `with_len` requires the length to describe live,
initialized elements in the allocation, and `as_str` requires those bytes to be
valid UTF-8. They preserve the data pointer's provenance and never round-trip
the address through an integer. `with_len` only accepts a thin pointer to a
sized pointee; a pointer that already carries metadata is `E3075`. `as_str`
only accepts `*raw [u8]` and `*raw mut [u8]`.

See [Raw pointers](raw-pointers.md#building-a-fat-pointer-from-a-thin-one) for
the full contract and the safe/unsafe boundary.

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

### C string literals

`c"..."` has type `*raw u8` and names a private, read-only, NUL-terminated
static byte array. It is a thin raw pointer, so it is FFI-safe and can be passed
directly to `const char *`. The terminator is not part of the source contents,
but it is always present; `c""` is valid and points at a single NUL.

```seal
const FORMAT: *raw u8 = c"value=%d";
let empty: *raw u8 = c"";
let escaped: *raw u8 = c"line\n\ttab \\ \"quoted\"";
let unicode: *raw u8 = c"olá";
```

The ordinary string escape table applies, including `\n`, `\t`, `\\`, `\"`,
`\xNN`, and `\u{...}`. A NUL written inside the literal is `E3090`: C would stop
at the first NUL and the rest would be unreachable. The bytes of a `*str` and a
`c"..."` literal share one interned image; the string carries a separate length
and ignores the terminator.

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

## 7. The safety contract

Every boundary crossing is unsafe because the compiler cannot inspect the
other side. An `unsafe` block is the place where the programmer accepts the
obligations below; the type checker verifies the shape of each value, never its
meaning.

### The declaration matches the definition

The symbol must exist in the final link, and its C prototype must match the
declared signature after the target ABI is applied. A mismatch is not a
compile error: it is undefined behavior at the call. The same obligation covers
`link_name`, function pointer types, and an `extern static`'s storage type.
Declare the narrowest correct type. A C `int` is `c_int`, not `i32` by
coincidence; a C `size_t` is `size_t`, not `usize` by coincidence. They are the
same type on the supported targets, but the C spelling records the contract and
survives a target whose widths differ.

### Every argument satisfies the C contract

A pointer must be non-null when C requires it, aligned for its pointee, valid
for the access C performs, and backed by initialized bytes. A length must
describe live elements, a format string must match the variadic arguments, and
a callback must remain callable for as long as C may call it. Seal's bounds,
initialization, and nullness checks stop at the boundary; after the call they
cannot describe what C did.

### Ownership stays with the owner

The C allocator and the Seal heap are separate contracts. Memory allocated by
C is released by the matching C function; memory from `seal_alloc` is released
with `seal_dealloc`, because a large alignment over-allocates and stores the
base pointer before the returned address. Passing a `seal_alloc` block to
`free`, or a `malloc` block to `seal_dealloc`, is undefined behavior.

The runtime allocation symbols are internal to the compiler and runtime. User
Seal code never declares or calls `seal_alloc`, `seal_alloc_zeroed`,
`seal_realloc`, or `seal_dealloc` directly. The hosted `std::alloc` module owns those private
declarations and exposes validated `Layout` values, `AllocError`, unsafe raw
wrappers, and `RawBuffer`. A block obtained through that module is still a Seal
block and must be released through the same module, never by C `free`.
`RawBuffer` owns one Seal block and releases it on `drop`; it does not own C
memory and cannot be used to adopt a `malloc` pointer.

A destructor runs only for a Seal value that Seal owns. When a value crosses
into C, no `Drop` implementation runs on the C side, and C storage that Seal
observes through `*raw` has no drop glue. An `extern struct` may implement
`Drop` for use inside Seal, but the destructor must not assume that C also runs
it. Conversely, a Seal value moved into a C structure is gone from Seal's point
of view: the program must not keep using or dropping the original.

### Pointers keep provenance across the call

The address and provenance of a pointer passed to C are preserved. C may
perform arithmetic the Seal type system cannot see, so the returned pointer is
only as valid as the C documentation says. `with_len` and `as_str` are the
supported way to rebuild a fat pointer; they require the length and the bytes
to describe a live allocation, and `as_str` additionally requires valid UTF-8.
Never reconstruct a fat pointer by combining an integer address with a length.

### Nothing unwinds across the boundary

Seal has no unwinding: a runtime panic aborts the process. C code reached
through `extern "c"` must not `longjmp` through a Seal frame, throw a C++
exception, or use any mechanism that unwinds the stack. A callback passed to C
therefore must not unwind either. The ABI is plain `"c"`; `"c-unwind"` is not
accepted (`E3080`), and the compiler never emits `nounwind` for a declaration
it does not own.

### C may call back at any time

A callback runs on C's schedule, not the caller's. It may arrive during the
same call, after it returns, or from another thread. The function must satisfy
its declared signature, must not unwind, and must not retain a borrowed Seal
reference beyond the lifetime its contract allows. Because a Seal function's
address escapes, the compiler suppresses `noalias` on that function: C may
alias through a path the whole-program proof did not model.

### The compiler adds no promises

An external declaration carries only the ABI-mandated attributes. The compiler
cannot assert `noalias`, `readonly`, `nounwind`, `noundef`, or `willreturn` for
code it does not own, and it does not. An external static is emitted as a
mutable global even when declared immutable, because C may write it at any
time; that is also why every access is unsafe.

### The unsafe checklist

Before writing an `unsafe` boundary, answer every question:

1. Does the symbol exist, and does its C prototype match this declaration
   exactly, including the ABI and the variadic form?
2. Is every pointer non-null, aligned, initialized, and valid for the access C
   performs? Is every length correct in elements or bytes, whichever C expects?
3. Who owns each allocation, and which function releases it? Does any `Drop`
   implementation run on the wrong side?
4. Can any call unwind, `longjmp`, throw, or abort across the boundary?
5. If a callback is registered, when and on which thread may it run, and what
   must stay alive until then?
6. Does the C contract permit the aliasing that Seal's `noalias` proof would
   otherwise forbid?

## 8. Exporting Seal functions to C

`export "c"` gives a Seal function a C symbol. The body stays ordinary Seal
code; the compiler emits a hidden internal function plus one C wrapper, so C
sees exactly the declared name and nothing from Seal's mangling scheme.

```seal
export "c" func add(a: i32, b: i32) -> i32 {
  a + b
}
```

```c
/* math.h */
int add(int a, int b);
```

```console
$ sealc build --emit=staticlib -o libmath.a math.sl
$ cc app.c libmath.a -o app
```

The wrapper uses the target's C ABI for every FFI-safe parameter and return
type, including C-layout structs returned in registers or through `sret`. The
C declaration must match the Seal signature exactly. A scalar `bool` is C's
`_Bool` (one `i1` value); a struct field that must occupy one byte uses
`c_bool`. Checked references, fat pointers, `char`, `i128`, and Seal-layout
aggregates are rejected with `E3084`, exactly as on an `extern` declaration.

An export must be a free function. It cannot be generic, cannot be named
`main`, cannot appear on an impl method, and cannot be declared in an import.
Its C name is the Seal function name; there is no `link_name` for exports. Two
exports with one name are `E3082`. A body with no export is never visible to
C, even when the function is `public`.

Calling an exported function from C does not create an `unsafe` obligation for
Seal: the C caller owns the arguments and must satisfy the same contract a C
function would. Seal cannot validate a null pointer, a dangling handle, or a
violated aliasing rule across that boundary; document those preconditions in
the header that accompanies the library. Exported functions must not unwind;
a Seal panic aborts instead of crossing into C.

Library artifacts are built directly from a source file or from a project
target:

```toml
[targets.math]
kind = "staticlib"   # or "cdylib"
sources = ["src/lib.sl"]
```

```console
$ seal build
$ ls seal/target/dev/libmath.a
```

A `staticlib` is `lib<name>.a` (`lib<name>.lib` on MSVC); it contains the
compiled target and the Seal runtime, so link it once. It does not embed native
libraries named by `link` directives; the final C link supplies them. A
`cdylib` is `lib<name>.so`, `lib<name>.dylib`, or `<name>.dll`; the linker adds
the platform's shared flag, links native libraries, and only declared exports
are dynamic symbols. `nm -D --defined-only libmath.so` therefore lists `add`
but not `seal.add`, `seal.main`, or the C entry wrapper. A library target has
no entry function and no C `main`.

Static archives are deterministic: object members are stored in a fixed order
and the archive tool writes no timestamps or temporary paths. GNU hosts use
`ar crsD`; Apple and BSD hosts use `ar crs` with `ZERO_AR_DATE=1`. Rebuilding
the same inputs produces identical bytes, and `seal verify-repro` compares
library artifacts like any other output.

## 9. Diagnostics

| Code | Meaning |
|------|---------|
| `E2001` | Two external functions declare the same Seal name |
| `E2068` | Two values declare the same Seal name |
| `E3070` | An external call appears outside `unsafe` |
| `E3080` | The ABI spelling is unknown or unavailable |
| `E3081` | The declaration is malformed, or uses an unsupported member |
| `E3082` | A linker symbol or exported name is empty, reserved, duplicated, or contains invalid characters |
| `E3083` | One symbol is declared with two different shapes |
| `E3084` | A parameter, return type, field, or static type is not FFI-safe, or an exported function is generic |
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
| `E6003` | The linker or archive tool failed; the message names the command and inputs |
| `E6005` | A library name or `-L` search path is invalid |

## 10. Current limits

The following are deliberately rejected, never silently miscompiled:

- Seal cannot define its own variadic function; only an `extern` declaration
  may end in `...` (`E3088`). A variadic function cannot be exported either.
- Exports are free functions only. They cannot be generic, cannot be methods,
  and cannot rename their C symbol; the function name is the C name.
- Only functions can be exported. Exported statics, constants, and types are
  not part of the surface, and the compiler does not generate a C header.
- A Seal function pointer is not a first-class value type: only the raw C form
  exists, and taking a Seal function's address is an explicit cast.
- Per-declaration attributes such as `noalias` or `noreturn` are not accepted
  on an external declaration.
- C aggregates by value on an architecture without an implemented aggregate
  ABI are rejected with `E6002`; x86_64 System V and Microsoft x64 are
  implemented.
- A C declaration cannot carry attributes, a different calling convention, or
  thread-local storage. The supported ABI spellings are `"c"`, `"cdecl"`, and
  `"system"`.
- There is no `extern enum`, union, or bitfield surface. Represent an enum as
  `c_int` plus constants, and a union or bitfield as the explicit integer
  storage it occupies.
- `c"..."` cannot contain an interior NUL, because C would stop at the first
  one (`E3090`).

These limits are visible in the type system or in a diagnostic; none of them
silently changes a layout or a call.

## 11. Patterns that stay auditable

### Put the contract in one function

Wrap a declaration group in safe functions whose bodies are the only `unsafe`
blocks that touch C. Callers then rely on one documented contract instead of
repeating the checklist.

```seal
extern type Handle;

extern "c" {
  func ffi_open(path: *raw u8) -> *raw mut Handle;
  func ffi_read(handle: *raw mut Handle, out: *raw mut u8, cap: usize) -> c_int;
  func ffi_close(handle: *raw mut Handle) -> ();
}

func read_first_byte(path: *raw u8) -> i32 {
  unsafe {
    let handle = ffi_open(path);
    if handle.is_null() { return -1; }
    let mut byte = [0u8; 1];
    let read = ffi_read(handle, &mut byte[0] as *raw mut u8, 1usize);
    ffi_close(handle);
    if read != 1 { return -1; }
    byte[0] as i32
  }
}
```

### Keep units in the name

`size` means bytes, `count` means elements, and `capacity` means the allocation
size. A C API mixes the three constantly; a name that records the unit removes
the most common boundary bug. Convert once, at the call, and do not reuse a byte
count as an element count.

### Validate before converting

Check a C return code, a null handle, and a reported length before turning them
into a Seal value. A failed C call usually returns a sentinel, not a Seal
`Result`; the wrapper is the right place to translate it.

### Never outlive the borrow

A pointer derived from a Seal place must not be stored in C beyond the place's
lifetime. If C must retain it, copy the bytes into C-owned storage, or keep the
ownership explicit with an opaque handle and a release function.

## 12. Comparison with Rust

The surface deliberately follows Rust's C interop, with Seal's smaller type
system and explicit unsafe gate:

| Concern | Rust | Seal |
|---------|------|------|
| Declarations | `extern "C" { fn f(x: c_int); static mut S: c_int; }` | `extern "c" { func f(x: c_int); static mut S: c_int; }` |
| Calling | `unsafe { f(1) }` | `unsafe { f(1) }` |
| ABI spelling | `"C"`, `"system"` | `"c"`, `"cdecl"`, `"system"` |
| C scalar names | `std::ffi::c_int` and friends | built-in `c_int` and friends |
| C-layout struct | `#[repr(C)] struct S { ... }` | `extern struct S { ... }` |
| Opaque type | `enum Opaque {}` behind a pointer | `extern type Opaque;` behind `*raw` |
| C string | `CStr`/`CString`, `c"..."` | `c"..."` with type `*raw u8`; no owning string type |
| Function pointer | `extern "C" fn(i32) -> i32` | `*raw extern "c" func(i32) -> i32` |
| Link request | `#[link(name = "m")]`, build script | `link "m";` in the block, or `-l m` |
| Variadic declaration | `fn printf(fmt: *const c_char, ...)` | `func printf(format: *raw u8, ...)` |
| Variadic definition | unstable | not part of the surface |
| Owning C memory | `Box::from_raw`, `CString` | explicit C free function; no owning wrapper |
| Unwinding | `"C-unwind"`, `catch_unwind` | never across the boundary; panics abort |

The main differences are consequences of scope: Seal has no owning C heap
types, so `CString`, `CStr`, `Box`, and `Vec` wrappers do not exist; `c"..."`
plus `with_len`/`as_str` covers the same ground. `std::alloc::RawBuffer` owns
untyped Seal allocations, not C memory, and it does not add a typed container.
Seal also has no `#[repr(C)]` attribute because C layout is a distinct
declaration form, `extern struct`, which cannot be confused with the default
layout.

## 13. Acceptance matrix

Every row below is exercised by the compiler's test suite or by
[`examples/045_ffi_libc.sl`](../../examples/045_ffi_libc.sl). The rows group the
73 acceptance cases into the areas a user can observe.

| Area | Verified behavior | Evidence |
|------|-------------------|----------|
| Scalar names | `c_int`, `c_long`, `size_t`, and `c_char` resolve from the target, not the host; `c_char` signedness and `c_long`/`c_wchar` widths match the C fixture at runtime | `crates/target/src/c_abi.rs`, `crates/hir/src/tests/c_types.rs`, `crates/codegen/tests/ffi.rs` |
| `_Bool` representation | a direct `bool` parameter and result use `i1`; `c_bool` occupies one byte in an aggregate; a `bool` field is `E3085` with a `c_bool` suggestion | `crates/codegen/src/tests/externs.rs`, `crates/typeck/src/tests/ffi.rs`, `crates/codegen/tests/ffi.rs` |
| C exports | scalar, 1-, 2-, and 3-field C-layout structs roundtrip through an exported function; a static archive links from C; a shared library exposes only the declared C names; a generic export, `main`, a duplicate name, and a non-FFI-safe parameter are `E3082`/`E3084` | `crates/codegen/tests/exports.rs`, `crates/typeck/src/tests/exports.rs`, `crates/build/tests/build.rs` |
| Declarations and calls | block and single-item declarations compile; `abs(-41)` returns 41; a call outside `unsafe` is `E3070`; `getpid` returns a non-zero pid | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/ffi.rs`, `examples/045_ffi_libc.sl` |
| Symbols | `link_name` selects the object-file symbol; `main` is `E3082`; two shapes for one symbol are `E3083`; leading underscores, digits, and long names are accepted | `crates/codegen/src/tests/externs.rs`, `crates/typeck/src/tests/ffi.rs` |
| External statics | a static is read and written in `unsafe`; an access outside it and a write to an immutable declaration are `E3086`; C sees a Seal write and Seal sees a C write | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/ffi.rs` |
| FFI safety | checked references, `char`, `i128`, fat pointers, and Seal-layout aggregates are `E3084`; thin raw pointers, scalars, and C-layout aggregates are accepted | `crates/typeck/src/tests/ffi.rs`, `docs/types.md` |
| Aggregate ABI | 1-, 2-, 3-, 4-, 8-, 16-, 24-, and 32-byte structs roundtrip by value, including nested structs, arrays, mixed integer/floating fields, and register versus `sret`/`byval` returns | `crates/codegen/tests/ffi.rs`, `crates/codegen/src/tests/externs.rs` |
| Opaque handles | `extern type` is created by C, passed behind `*raw mut`, and freed by C; by-value use and dereference are `E3087`; `c_void` follows the same rule | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/ffi.rs` |
| Variadics | zero, one, and eight extra arguments; `i8`/`u8`/`i16`/`u16`/`bool` promote to `c_int`, `f32` to `f64`; aggregates, `char`, strings, and fat pointers are `E3088`; malformed declarations are `E3088` | `crates/codegen/tests/ffi.rs`, `crates/codegen/src/tests/externs.rs` |
| C strings | `c""`, escapes, and multibyte UTF-8 are NUL-terminated; an interior NUL is `E3090`; a `*str` initializer and a `c"..."` literal share the interned image | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/ffi.rs` |
| Function pointers | Seal functions become C callbacks through an explicit cast; `qsort`, `atexit`, aggregate callbacks, variadic pointers, negative returns, and null pointers work; a mismatch, a mutable pointer, or a call outside `unsafe` is `E3089` | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/ffi.rs` |
| Buffers and fat pointers | `with_len` and `as_str` build slice and string pointers from a C buffer; a wrong receiver is `E3075`; both require `unsafe` and are rejected in constants | `crates/codegen/tests/ffi.rs`, `crates/typeck/src/tests/raw_methods.rs`, `crates/codegen/src/tests/fat_pointers.rs` |
| Linking | source `link`, `-l`/`-L`, and `--link-arg` all link and run; libraries resolve in declaration order, including one library depending on the next; a relative `-L`, a path with spaces, and an extra object are accepted; invalid names and paths are `E6005`; a missing library is `E6003` and names it | `crates/codegen/tests/ffi.rs` |
| Constant evaluation | an external call in a constant is `E4033`; an external static read is `E4034`; a `c"..."` initializer is accepted | `crates/const-eval/tests/errors.rs` |
| IR contract | a declaration has no `noalias`, `readonly`, or `nounwind`; a variadic call uses the promoted types; an external global is `external global` without `constant`; a program without `extern` emits no declarations | `crates/codegen/src/tests/externs.rs` |
| Determinism and targets | `-O0` and `-O2` agree; `--emit=obj` with `extern` compiles for another architecture; object and assembly output ignore link inputs | `crates/codegen/tests/ffi.rs`, `crates/codegen/tests/end_to_end.rs` |

## 14. Complete example

The chapter's canonical program is [`examples/045_ffi_libc.sl`](../../examples/045_ffi_libc.sl).
It declares `abs` and `snprintf`, formats `value=%d` from a `c"..."` literal,
checks the bytes C wrote, and returns `7`:

```seal
extern "c" {
  func abs(value: c_int) -> c_int;
  func snprintf(buffer: *raw mut u8, size: usize, format: *raw u8, ...) -> c_int;
}

func main() -> i32 {
  let mut buffer: [u8; 16] = [0u8; 16];

  unsafe {
    let value = abs(-7);
    if value != 7 { return 1; }

    let written = snprintf(
      &mut buffer[0] as *raw mut u8,
      16usize,
      c"value=%d",
      value,
    );
    if written != 7 { return 2; }
    if buffer[0] != 118 { return 3; }
    if buffer[5] != 61 { return 4; }
    if buffer[6] != 55 { return 5; }
    if buffer[7] != 0 { return 6; }
  }

  7
}
```

It compiles at `-O0` with debug information and at `-O2` without it, and the
acceptance suite asserts the exit status in both modes.

## 15. Quick reference

```text
extern "ABI" { link "name"; func f(x: T, ...) -> R link_name "symbol"; static [mut] s: T; }
extern "ABI" func f(x: T) -> R;
extern "ABI" static [mut] s: T;
extern struct S { field: T, ... }
extern type Opaque;
*raw extern "ABI" func(T, ...) -> R
c"bytes"
```

| Operation | Spell it as |
|-----------|-------------|
| Declare a C function | `extern "c" func name(args) -> R;` |
| Export a Seal function | `export "c" func name(args) -> R { ... }` |
| Rename the symbol | `... link_name "symbol";` |
| Declare a C variable | `extern "c" { static mut name: T; }` |
| Declare a C struct | `extern struct S { field: T }` |
| Declare an opaque handle | `extern type S;` and use `*raw mut S` |
| Name a library | `link "m";` in the block, `-l m`, or `-L DIR` |
| Build a C library | `sealc build --emit=staticlib` or `--emit=cdylib`; `kind = "staticlib"`/`"cdylib"` in a target |
| Pass a linker argument | `--link-arg ARG` |
| Call C safely | `unsafe { name(args) }` |
| Pass a NUL-terminated string | `c"text"` |
| Take a function pointer | `func_name as *raw extern "c" func(T) -> R` inside `unsafe` |
| Build a slice from C data | `pointer.with_len(count)` inside `unsafe` |
| Read it as text | `slice.as_str()` inside `unsafe` |
| Release C memory | call the C free function; never `seal_dealloc` |
| Release Seal memory | the matching `seal_dealloc`; never C `free` |

| Gate | Codes |
|------|-------|
| Malformed declaration or symbol | `E3080`–`E3083`, `E3091`–`E3093` |
| Unsafe or FFI-safety violation | `E3070`, `E3084`–`E3090` |
| Constant-evaluation violation | `E4033`, `E4034` |
| Link or configuration failure | `E6003`, `E6005` |

For the type-level rules behind these tables, read [`../types.md`](../types.md);
for ownership and provenance across the boundary, read
[`../memory-model.md`](../memory-model.md).
