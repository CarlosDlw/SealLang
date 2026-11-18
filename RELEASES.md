# Release notes

## Unreleased

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
