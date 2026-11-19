# Release notes

## Unreleased

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
