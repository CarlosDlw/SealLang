# Release notes

## Unreleased

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
