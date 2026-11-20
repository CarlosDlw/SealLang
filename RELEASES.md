# Release notes

## Unreleased

### First-class functions and closures

Functions are now values, and closures are part of the language:

- `func(A, B) -> R` is a one-word, `Sized`, `Copy` function pointer with the
  internal Seal ABI. `unsafe func(...)` carries the caller contract in the type.
  A safe function converts to a safe or unsafe pointer; unsafe never converts
  back, and no signature or ABI may change silently (`E3098`). Function pointers
  have no null value, equality, arithmetic, or implicit C conversion.
- Closures are written `|x| ...`, `|| ...`, and `|x: i32| -> i32 { ... }`, with
  optional `move`. They capture whole variables by reference by default and by
  value with `move`. Captures live inline; there is no boxing, reference
  counting, or allocation.
- The body's use of a capture selects the callable kind: `Fn` reads, `FnMut`
  writes, and `FnOnce` consumes. `Fn: FnMut: FnOnce`, and generic bounds accept
  closures and safe function pointers without a dictionary or runtime search.
  An explicit `f.call(...)` is rejected; the call operator is the surface.
- A capture-free closure converts to a matching `func` through an internal
  trampoline. A closure with captures is `E3098`; `dyn Fn`/`impl Fn`, explicit
  capture lists, closure patterns, and implicit C interop are out of scope.
- Closures follow the ordinary ownership rules: an exclusive capture makes the
  closure non-`Copy`, a non-`move` body cannot move a capture out (`E3095`), a
  borrowed capture keeps its loan live (`E5001`), and an `FnOnce` environment
  drops each field once. Function addresses and indirect calls are `E4033` in a
  constant context.
- `move` is a reserved keyword. `Fn`, `FnMut`, and `FnOnce` are compiler traits
  that cannot be declared or implemented (`E3097`).
- `examples/046_function_pointers.sl`, `examples/047_closures.sl`, and
  `examples/048_function_traits.sl` exercise the surface. The complete guide is
  [`docs/language/first-class-functions.md`](docs/language/first-class-functions.md).

### C interoperability (FFI)

Seal now declares and calls C code directly, without a header parser or a
binding generator:

- `extern "c" { func f(x: c_int) -> c_int; static mut value: c_int; }` declares
  functions and storage in a block; the single-item form declares one function
  or static. `"cdecl"` and `"system"` are accepted as the same calling
  convention. `link "m";` names a native library, and `link_name "abs"`
  selects the linker symbol independently of the Seal name.
- C scalar names (`c_int`, `c_char`, `c_long`, `c_wchar`, `size_t`, and the
  rest) resolve from the target, not the host. `c_bool` is the one-byte `_Bool`
  field representation; a direct `bool` uses LLVM `i1` and is FFI-safe. A
  `bool` field in an `extern struct` is rejected with a suggestion to use
  `c_bool`.
- `extern struct` is an ordinary aggregate with C layout: declaration-order
  fields, C offsets and padding, nested structs and arrays, and by-value calls
  coerced with the target ABI. On System V AMD64, small structs use
  integer/SSE pieces, large ones use `byval`/`sret`, and exact widths such as
  `i24` are preserved. Microsoft x64 uses its 1/2/4/8-byte rule and indirect
  passing for every other size. `extern type` and `c_void` are opaque: they
  exist only behind `*raw`.
- A variadic declaration ends in `...` and applies C's default promotions
  (`bool`, `i8`, `u8`, `i16`, `u16` to `c_int`; `f32` to `f64`). `c"..."` is a
  read-only NUL-terminated `*raw u8`, and an interior NUL is `E3090`.
- C function pointer types, `*raw extern "c" func(...) -> R`, accept Seal
  functions through an explicit cast. The compiler emits a cached C-ABI
  trampoline when the internal signature differs, and suppresses `noalias`
  once the function's address escapes. `qsort`, `atexit`, aggregate and
  variadic callbacks work. `with_len` and `as_str` build slice and string
  pointers from a C buffer.
- Every external call, external static access, and indirect call requires an
  `unsafe` block. Constant evaluation rejects an external call (`E4033`) and
  an external static read (`E4034`), while a `c"..."` initializer is accepted.
- Native libraries come from a source `link` directive, `-l NAME`, or
  `--link-arg ARG`, with `-L DIR` for search paths. The order is deterministic,
  each value is one argv element, invalid names and paths are `E6005` before any
  temporary file is written, and a failed link is `E6003` with the command and
  libraries.
- `examples/045_ffi_libc.sl` calls `abs` and `snprintf` and returns 7. The
  complete guide, including the safety contract and the acceptance matrix, is
  [`docs/language/ffi.md`](docs/language/ffi.md).

### Opt-in `Copy` for structs and enums

Structs and enums can now opt into bitwise duplication with an explicit
implementation:

```seal
struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  let copy = point;
  drop(point); // no-op: `Point` is `Copy`
  copy.x + point.y
}
```

- The implementation covers the whole declaration. It repeats the declaration's
  parameters in order, cannot specialize an instantiation, and may appear only
  once for a type. A missing, extra, or reordered parameter is `E3076`.
- Every field of every variant must be `Copy`. A type parameter needs a `Copy`
  bound; references and raw pointers do not constrain their pointee. A failed
  field is `E3077` and names the field or variant.
- `Copy` and `Drop` are mutually exclusive (`E3068`). A `Copy` type has no drop
  sites, drop flags, glue, or vtable entry for a destructor.
- Assignment, arguments, returns, field and element reads, match and `let`
  bindings, `for` items, repeated arrays, statics, and equality all read a
  `Copy` value without moving it. `drop(copy)` is a no-op and the value remains
  usable.
- Copying an aggregate copies the checked loans of its reference fields; a raw
  pointer field copies only its address. Layout and ABI are unchanged: MIR
  chooses `Operand::Copy` instead of a move, and code generation emits no new
  item.

`examples/044_copy_optin.sl` exercises the surface at `-O0` and `-O2`. The
complete guide is [`docs/language/copy.md`](docs/language/copy.md).

### Raw pointers and the unsafe layer

Seal now has nullable raw pointers with provenance, pointer arithmetic, and an
explicit unsafe boundary:

- `*raw T` is read-only and `*raw mut T` permits writes. Both are `Copy`,
  `Sized`, have no destructor, never unify with `*T`/`*mut T`, and are one word
  for a sized pointee or two words for `str`, a slice, or a trait object.
- `&place as *raw T`, `&mut place as *raw mut T`, and the permission-discarding
  downgrade are accepted. Raw-to-raw casts may change the pointee or
  mutability, and only `usize`/`isize` convert to and from a thin pointer.
  Shared references cannot gain write permission (`E3074`).
- Raw pointer equality compares the data address. `is_null`, `addr`,
  `expose_addr`, `with_addr`, `len`, and `data` are safe queries; `data`
  discards slice/string/trait-object metadata and must be called before
  arithmetic on a fat pointer.
- Dereference, writes, copies, non-wrapping arithmetic, `offset_from`, and
  calls to `unsafe func` require an `unsafe { ... }` block (`E3070`). Writes
  require `*raw mut T` (`E3075`); `unsafe` does not relax any other check.
- `read`/`write`, unaligned forms, `copy` (memmove), and
  `copy_nonoverlapping` (memcpy) lower to the expected memory operations.
  Inbounds arithmetic uses inbounds GEPs, wrapping arithmetic uses the
  non-inbounds form, and internal offsets never round-trip through integers.
- Constants may store raw pointers and read through pointers to compiler-owned
  allocations. Address observation is `E4030`, writes are `E4031`, and null,
  misaligned, out-of-bounds, or uninitialized reads are `E4032`.
- `examples/043_raw_pointers.sl` exercises the full surface at `-O0` and `-O2`.

### Destructors

A type may now declare `impl Drop` and take ownership of its release. The
destructor runs at scope exit, before the fields the type owns, and the built-in
`drop(value)` releases a value early:

```seal
struct Guard {
  id: i32,
}

impl Drop for Guard {
  func drop(self: *mut Guard) {
    // Release whatever `id` names.
  }
}

func main() {
  let guard = Guard { id: 1 };
  drop(guard); // runs the destructor here
}
```

Field drops follow declaration order, an enum drops only its active variant, an
array drops elements in index order, and a value moved into another function or
returned from one is released by its new owner. `impl Drop` is checked: one
implementation per type, covering the declaration's parameters, for a type that
is not `Copy`. `Drop` is not a bound or a trait object, and the destructor
cannot be called directly.

A `static` is never released, a constant cannot run a destructor (`E4028`), and
a panic ends the program without unwinding, so destructors do not run during a
panic.

### `const`, `static`, and `unsafe` are reserved keywords

Global data is now part of the language, and the three keywords it needs are
reserved by the lexer in every language version:

| Keyword | Role |
|---------|------|
| `const` | constant item, block-scope constant, or `const func` |
| `static` | static item, block-scope static, or `static mut` |
| `unsafe` | prefix for the block that unlocks `static mut` places |

Seal is pre-1.0, so this is a deliberate breaking change: an existing program
that used any of these words as an identifier no longer parses.

**Migration.** Rename the identifier. The diagnostic points at the token and
names the reserved word:

```seal
// Before: `static` and `unsafe` were ordinary names.
func read(static: i32) -> i32 {
  let unsafe = static;
  unsafe
}

// After.
func read(state: i32) -> i32 {
  let unchecked = state;
  unchecked
}
```

There is no silent fallback and no compatibility mode: a reserved word is never
read as a binding or a parameter, because that would make the same spelling mean
two different things in two programs.

### Global data

- `const` and `static` items may appear at file scope or in a block. Both are
  evaluated at compile time; a `static` is one allocation for the whole program
  and a `const` is a value substituted at every use.
- `const func` may be called from a constant initializer and at runtime. The
  evaluator rejects non-deterministic operations and enforces step, depth,
  allocation, and size limits.
- A static may hold aggregates, references, slices, strings, and trait objects.
  Their addresses are relocations resolved at compile time, so there are no
  global constructors and no initialization order.
- Only `static mut` is writable. Reading, writing, borrowing, or projecting it
  requires an `unsafe { ... }` block, and a reference to it may not escape the
  function that created it.

### Structural equality

`==` and `!=` now compare the values they are given instead of failing in code
generation:

- Structs compare field by field, enums compare the tag and the active variant's
  fields, and arrays and tuples compare their elements.
- `*str` compares length and bytes; `*[T]` compares length and elements.
- Both operands are read, never moved, so `a == b` leaves a non-`Copy` value
  usable.
- Thin references and trait objects are rejected with `E3002` instead of
  reaching the backend: a reference only has an address, and a trait object
  erases the type that could be compared.

Constant evaluation follows the same rules, so a comparison in a `const` or
`static` initializer produces the value the same comparison produces at runtime.
