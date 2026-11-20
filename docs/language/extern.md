# External functions and statics

Status: declared functions, calls, and static storage are implemented in the
current language version. Variadic declarations, external aggregates, opaque
types, `link` directives, and function pointers belong to later phases and are
rejected with a diagnostic instead of being silently accepted.

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
extern "ABI" { item* }
extern "ABI" func name(parameters) -> Return link_name "symbol";
extern "ABI" static [mut] name: Type;
```

- The ABI is always a string. `"c"`, `"cdecl"`, and `"system"` are accepted and
  all mean the target's default C convention. Any other spelling, including
  `"stdcall"` and `"fastcall"`, is `E3080` because the compiler cannot emit it.
- `func` declarations have names, typed parameters, and an optional return
  type. There is no receiver, body, generic parameter list, `where` clause, or
  `unsafe` marker: an external function is implicitly unsafe to call.
- `static` declarations have a name and a type. `static mut` is the only form
  Seal may write.
- A member of a block inherits the block's ABI. A member cannot itself be an
  `extern` block (`E3092`).
- `extern` is contextual. Outside item position it is an ordinary identifier,
  and a function or binding may be named `extern`.

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
- An external static may have any declared type that a Seal static may have;
  accessing it safely is the caller's responsibility.

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

## 6. Code generation and linking

Each declaration produces exactly one LLVM declaration, cached by its HIR
definition:

- Functions use `external` linkage, the target's C calling convention, and no
  attributes. The compiler does not own the body, so it cannot promise
  `noalias`, `readonly`, `nounwind`, or any other property.
- Statics use `external` linkage and are never marked `constant`, because C may
  initialize them at load time. A `static mut` declaration produces writable
  storage; an immutable one is still emitted as a mutable global.
- A `void` external call does not write a destination.
- Debug builds describe the declaration with a subprogram that has no body.

The program fails at link time, not at compile time, when the symbol is missing
from every linked library. Resolving libraries explicitly (`link` directives
and `-l`/`-L`) belongs to a later phase; the C runtime's own symbols are
available without flags on the supported targets.

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
| `E3084` | A parameter, return type, or static type is not FFI-safe |
| `E3086` | An external static is accessed outside `unsafe`, or an immutable one is written |
| `E3088` | A variadic declaration was written; variadics are not implemented |
| `E3091` | A `link` directive was written; explicit libraries are not implemented |
| `E3092` | An `extern` declaration is nested inside another `extern` block |
| `E3093` | An external function declares generic parameters |
| `E4033` | A constant calls an external function |
| `E4034` | A constant reads an external static |

## 8. Current limits

The following are deliberately rejected, never silently miscompiled:

- Variadic declarations and calls (`E3088`).
- `extern struct`, `extern type`, and `c_void` (`E3081`; opaque C types arrive
  with the aggregate phase).
- `link` directives and per-library linker flags (`E3091`).
- Function pointers and callbacks (rejected as a non-FFI-safe type).
- Per-declaration attributes such as `noalias` or `noreturn`.

## 9. Complete example

```seal
extern "c" {
  func abs(value: i32) -> i32;
  func getpid() -> i32;
  func srand(seed: u32) -> ();
}

func main() -> i32 {
  unsafe {
    srand(7u32);
    if abs(-41) != 41 { return 1; }
    if getpid() <= 0 { return 2; }
  }
  42
}
```

A correct run exits with `42`. The example is exercised at `-O0` with debug
information and at `-O2` without it.
