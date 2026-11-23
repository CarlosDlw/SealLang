# Static semantics

This chapter defines which syntactically valid programs are accepted, what
types and names they have, and which static obligations they must satisfy. It
covers name resolution, type formation, typing, ownership and initialization,
borrowing, pattern coverage, `unsafe`, constant evaluation, and C interop.

The detailed type, layout, and ownership tables in [`../types.md`](../types.md)
and [`../memory-model.md`](../memory-model.md) are normative companions. The
rules below are the acceptance contract; the tables are the complete enumerations
those rules refer to.

## Programs, packages, and modules

**S1. Program unit.** A compilation unit is a package. A package contains one or
more source files, each belonging to exactly one module. A file without a
`module` declaration belongs to the package root. A file with
`module a::b;` belongs to `a::b`; parent modules are implicit.

**S2. Module fragments.** Several files may declare the same module. Their items
share one namespace, so two fragments cannot declare the same name in the same
namespace (`E2083`/`E2102`). An empty module is valid.

**S3. Imports.** An import path is absolute. Its first segment is `root`, a
top-level module of the current package, or a dependency alias from the compile
plan. Every middle segment MUST name a module; the last may name a module or an
item. A group expands to one binding per item; `as` renames the binding. Imports
are file-scoped and are resolved before bodies, so module cycles are legal.

**S4. Import validity.** A missing root is `E2073`, a missing submodule or item
is `E2070`/`E2002`, a module/item ambiguity is `E2074`, two imports binding the
same local name are `E2078`, and an import colliding with an item or submodule
is `E2077`. A glob is `E2076`. An unused import is the `unused_import` warning,
not an error.

**S5. Re-exports.** `public use` and `internal use` alias a target through the
declaring module instead of binding a local name. An alias never creates a new
definition: types keep canonical identity. Alias chains are resolved to a fixed
point; a cycle is `E2089`. A re-export cannot widen visibility.

**S6. Visibility.** `private` (the default) is visible in the declaring module
and its descendants; `internal` is visible anywhere in the package; `public` is
visible to other packages when the declaring module is exposed by the package
API. Modules themselves are always internal to their package and take no
visibility modifier. Variants inherit their enum's visibility and trait methods
inherit their trait's. A field has its own visibility. A public field whose type
is not public is `E3104`.

**S7. Lookup order.** Unqualified lookup is per namespace and proceeds from the
innermost local scope outward: locals, the current module's items, the file's
explicit imports, then the prelude. An explicit import that shadows a prelude
name wins without a collision. A name in the wrong namespace is not a candidate:
a type name does not resolve as a value, a module is not a value (`E2086`), and
a trait name is neither a type nor a value (`E2027`).

**S8. Prelude.** A plan may select one prelude module. Its public item exports
are visible in every file without an import. Modules are not injected
(`E2101`); duplicate prelude names in one namespace are `E2102`; a prelude cycle
is `E2103`. `implicit-prelude = false` and `--no-prelude` disable injection.

## Namespaces, scopes, and definitions

**S9. Namespaces.** There are three namespaces: value (functions, constants,
statics, extern functions, extern statics, locals, parameters, captures), type
(structs, enums, opaque types, type parameters, primitive names, aliases), and
trait (traits). A name may exist in more than one namespace without colliding;
a duplicate in the same namespace is an error.

**S10. Local scopes.** A `let` inserts its name after its initializer, so the
binding is not visible in its own initializer. `if`/`while`/`for` bodies do not
leak bindings. An `if let` pattern is scoped to its then block and never its
else branch. A `while let` pattern is scoped to the loop body. A `for` pattern
is scoped to the body. A closure body sees its captures under `Res::Capture`,
never as a local of the enclosing body.

**S11. Definitions.** Every declaration has one stable definition identity. A
function owns one body; a trait default owns one body; a constant and a static
own one initializer. Items are unique per module and namespace. Inherent methods
and associated functions share one namespace per type (`E2017`). `self` is only
a receiver name and MUST be the first parameter when present (`E2016`).

**S12. Item constraints.** A `func` has a body; a trait method has a body or
`;`; an `extern` function has no body or generics. `const` is never `mut`;
`static` takes at most one `mut`. A `const` and `static` require both a type and
an initializer. A block-scope `const func` is not a statement (`E1014`).

**S13. Module position.** A module declaration is at most once and first in its
file; imports precede every non-import item. A module declaration takes no
visibility, `test`, `bench`, or `deprecation` modifier (`E2079`). An `impl` and
an `extern` block take no deprecation (`E2079`). An import takes no `private`,
`test`, `bench`, or `export` (`E2079`).

## Types

**S14. Type kinds.** The type language contains the primitive scalars, `bool`,
`char`, `str`, shared and exclusive references, raw pointers, tuples, arrays,
slices, trait objects, Seal function pointers, C function signatures behind raw
pointers, nominal structs and enums, opaque external types, type parameters,
closures, and the internal `Error`/`Infer` types. `()` is the empty tuple. `!`
is the never type and has no type-position spelling in `0.1`. A type alias is
not a type kind of its own: it is a declaration that expands to one of these
kinds (S61–S65).

**S15. Well-formedness.** Every stored value MUST be `Sized`: locals,
parameters, return values, fields, tuple elements, array elements, and enum
payloads. `str`, `[T]`, and `dyn Trait` are unsized and appear only behind a
reference or raw pointer. A struct or enum cannot contain itself by value
(`E3016`). An array's total size must fit the target address space (`E3023`).
A type parameter is `Sized` by default; there is no `?Sized`.

**S16. References.** `*T` is shared, non-null, non-owning, and `Copy`; `*mut T`
is exclusive, non-null, non-owning, and `Copy`. They never unify. `*str` and
`*[T]` are two-word fat references; `*dyn Trait` is `{ data, vtable }`. A
reference's pointee is always live and initialized for the loan's lifetime.
There is no lifetime syntax and no region parameter in a type.

**S17. Raw pointers.** `*raw T` and `*raw mut T` are nullable, `Copy`, `Sized`,
and provenance-carrying. A sized pointee gives one word; `str`, a slice, or a
trait object gives two words. Raw pointers never unify with references. A
mutable raw pointer may be cast to a shared one; the reverse is not a safe
widening. `*raw extern "c" func(...)` is a read-only C function pointer.

**S18. Nominal and structural aggregates.** A struct and enum are nominal:
their identity is the declaration, not their fields. A tuple is structural. A
struct or enum is `Copy` only with an explicit `impl Copy` covering the
declaration and every field of every variant; a tuple is `Copy` exactly when
every element is; an array is `Copy` when its element is. `Copy` and `Drop` are
mutually exclusive (`E3068`).

**S19. Type properties.** `Copy`, `Sized`, and `needs_drop` are derived as in
[`../types.md`](../types.md#type-properties). A struct or enum is `Sized` when
every field of every variant is; a tuple when every element is; an array when
its element is. A trait object's `needs_drop` comes from its vtable drop slot.

**S20. Equality and layout.** Type equality is identity after interning. Layout
is deterministic: declaration order, natural alignment, no reordering; enum tag
first, then aligned payload; arrays and tuples as specified in
[`../types.md`](../types.md#layout). Layout is part of the language, not the host
C ABI, except for `extern struct`.

**S21. Generic declarations.** A generic declaration introduces type parameters
checked once against rigid parameters. Every use instantiates with concrete
arguments and emits one instance per reachable `(definition, arguments)` pair.
A type annotation MUST spell every argument; expression arguments may be
inferred. An argument that cannot be inferred is rejected, never defaulted.
Bounds are `Copy` or trait bounds. There is no specialization, associated type,
const generic, default argument, or `?Sized`.

**S22. `Self`.** `Self` is the implementing type in a trait or impl. It is
rejected in a free function (`E2025`). A trait receiver uses `Self`, `*Self`, or
`*mut Self`; an impl writes the concrete type. `Self` in a non-receiver
parameter or return type is allowed only for static dispatch; it prevents the
trait from being object-safe.

## Traits and implementations

**S23. Trait contracts.** A trait declares methods with optional default bodies.
An impl MUST provide every method without a default (`E2022`), provide nothing
else (`E2023`), and match the receiver, parameters, and return type after
substitution (`E2024`). A default body is checked once with `Self` and the
implicit `Self: Trait` bound.

**S24. Coherence.** A type implements a trait once. An identical header is
`E2021`; overlapping headers are `E2058`; bounds do not make impls disjoint.
The orphan rule is per package: an impl is valid only when the trait or the self
type is local to the package (with the package's declared foreign exceptions).
Supertrait cycles are `E2064`.

**S25. Dispatch.** Lookup order is inherent method, type-parameter bound, trait
impl for a concrete type, then the intrinsic catalog. Two equally applicable
trait methods are ambiguous (`E2028`). A call through a type parameter records a
symbolic trait method for monomorphization. A call through `*dyn Trait` loads
the method from the vtable; it never searches at runtime.

**S26. Object safety.** A trait is usable after `dyn` only when every method has
a `*Self` or `*mut Self` receiver, no `Self` in another parameter or return
type, no generic method parameters, and no associated function. Supertraits must
also be object-safe. The violation is reported when the `dyn` type is formed
(`E2065`), not when the trait is declared.

**S27. Trait-object coercion.** At an expected `*dyn Trait`/`*mut dyn Trait`
type, a thin reference to a concrete `S` coerces when `S` implements `Trait`.
The reference kind is preserved: `*mut S` never coerces to `*dyn Trait`
(`E3047`); reborrow shared explicitly. Coercion copies two words and allocates
nothing.

## Expressions and typing

**S28. Coercions.** Checked types are equal, except that `!` and `Error` coerce
to any type, a thin pointer to a sized concrete value coerces to a trait-object
pointer of the same kind, a function item coerces to a Seal `func` pointer with
the same signature, and an array reference coerces to a slice reference. A
`*mut T` reborrows as `*T` where a shared receiver is expected. There is no
implicit numeric conversion and no implicit deep copy.

**S29. Literals.** An unsuffixed integer literal defaults to `i32`; an
unsuffixed float defaults to `f64`. The expected numeric type is adopted when
the literal belongs to the same family; a suffix always wins. A literal must fit
its selected type (`E3011`). Boolean, character, string, and C-string literals
have types `bool`, `char`, `*str`, and `*raw u8`.

**S30. Arithmetic.** `+ - * / %` require both operands to have the same integer
or float type. Integer arithmetic wraps in two's complement, with no overflow
check. Division or remainder by zero panics, as does `MIN / -1` for signed
integers. Shift amounts are masked to the operand width. Unary `-` accepts a
signed integer or float and wraps for the minimum signed value; it is rejected
for unsigned integers. `!` requires `bool`; `~` requires an integer. Bitwise
operators require integers.

**S31. Comparisons and logic.** `==`/`!=` follow the structural equality table
in [`../types.md`](../types.md#equality); references, trait objects, function
pointers, and closures are not comparable. Ordering is defined for numeric
scalars and `char`; floats follow IEEE ordering, and `bool` has no ordering.
`&&`/`||` require `bool` and short-circuit. Conditions of `if` and `while`
require `bool`; there is no truthiness.

**S32. Places and assignment.** A place is a local, parameter, const, static,
dereference, field, tuple field, index, or a parenthesized place. Assignment
requires a mutable place; a `let` binding is immutable unless `mut`, a `*T`
cannot be written, a `const` place is never writable, and a `static mut` place
requires `unsafe`. A simple assignment evaluates its value before the target
place; a compound assignment or update evaluates the target place once before
the value. The target of `++`/`--` must be a writable numeric or pointer place.

**S33. Calls.** A direct call requires the arguments to match the callee's
parameter types exactly, after inference and coercions. The count must match;
an external variadic call accepts additional arguments of FFI-safe types. A
method call checks the receiver form, then the arguments; an associated function
is called through `Type::name`, not as a method (`E2019`). A call to an
`unsafe func` requires an enclosing `unsafe` block.

**S34. Blocks and control flow.** A block's value is its tail expression, or
`()` if there is none. `return` has the function's return type; `break` and
`continue` have type `!`; `if` branches unify to one type, with `!` coercing. An
`if` without `else` has type `()`. `match` arms unify similarly. Divergence does
not make later statements unreachable for type checking.

**S35. Struct and enum construction.** A struct literal MUST initialize every
field exactly once; unknown, missing, and duplicate fields are
`E2006`–`E2009`. A positional variant receives exactly one argument per field in
declaration order (`E2038`). Field visibility is checked at the literal. A
unit variant is a path value; a named or positional variant is constructed with
the corresponding literal/call form.

**S36. Arrays, slices, and indexing.** An array literal's elements unify to one
element type. `[value; N]` requires `value: Copy`, checked even when `N == 0`;
`[]` needs an expected array type (`E3019`). Built-in indexing requires `usize`,
checks constant bounds at compile time (`E3022`), and runtime bounds before the
access. A runtime out-of-bounds index panics with status 101. A custom
`Index`/`IndexMut` implementation is selected when the base is not an array or
slice; a write requires `IndexMut` (`E3054`).

**S37. Closures.** A closure's environment holds one field per captured
variable in first-mention order. A default capture is a shared reference for a
read or an exclusive reference for a write; `move` captures the value. The
closure kind is `Fn`, `FnMut`, or `FnOnce` according to its use of captures. A
closure with captures does not coerce to `func` (`E3098`); a capture-free one
does. Calling a closure follows the corresponding `Fn`-family bound.

**S38. Casts.** `value as Type` is accepted only for the numeric pairs and
pointer pairs in [`../types.md`](../types.md#explicit-numeric-casts) and the raw
pointer rules. Numeric casts are infallible, pure, and do not move a non-`Copy`
operand. A cast involving a type parameter is rejected (`E3049`); `bool`,
`char`, enums, aggregates, slices, arrays, tuples, trait objects, `str`, and
`()` have no numeric cast (`E3048`). Pointer-to-integer and integer-to-pointer
casts are only through `usize`/`isize` and never make an address usable as a
checked reference.

## Ownership, initialization, and borrowing

**S39. Ownership.** Every value has exactly one owner. Assignment, argument
passing, and return move unless the type is `Copy`. A moved-from place cannot be
read, assigned, or dropped again. Moving one field or element out of a non-`Copy`
aggregate leaves the siblings initialized and usable; moving the whole aggregate
moves every part. A `Copy` read duplicates bits and leaves the source usable.

**S40. Definite initialization.** Reading a local requires initialization on
every path that reaches the read. Assignments in both branches of an `if` count
only at the join; a diverging branch does not participate. A `while` or `for`
restores the state from before the loop. An uninitialized immutable binding may
be initialized once; reassignment requires `mut`.

**S41. Destruction.** A live owner is dropped exactly once at scope exit, on
`return`, `break`, `continue`, and when overwritten. Locals drop in reverse
declaration order; parameters follow the body's locals; struct fields drop in
declaration order after the struct's destructor; an enum drops only its active
variant; an array drops elements in increasing index order. A move transfers the
obligation. `drop(value)` consumes and drops immediately; `mem::forget(value)`
consumes and deliberately leaks, running nothing. A `static` is never dropped.
A constant cannot drop a value that needs a destructor (`E4028`).

**S42. Borrowing.** `&place` creates a shared loan of type `*T`; `&mut place`
creates an exclusive loan of type `*mut T`. Shared loans may overlap and allow
reads; an exclusive loan conflicts with every other loan and with reads, writes,
and moves of the same place. A loan's region is inferred from liveness. A
reference may not outlive the storage it borrows. `*mut T` reborrows as `*T` for
a shared receiver.

**S43. Global effects.** A call applies the transitive summaries of the
mutable statics it may read or write. A summarized read conflicts with a live
exclusive loan (`E5011`); a summarized write conflicts with any live loan
(`E5013`). A loan into `static mut` may not escape the function that created it
(`E5012`).

**S44. `Copy` and `Drop`.** A type may implement `Copy` only when every field of
every variant is `Copy` and any type parameter has a `Copy` bound. A type may
implement `Drop` at most once and only for a nominal aggregate. `Copy` and
`Drop` never hold together. Neither implementation changes layout or ABI.

## Patterns and `match`

**S45. Pattern typing.** A pattern is checked against the scrutinee type. A
binding introduces a new local; a `@` pattern binds the whole matched value and
then applies the subpattern. A literal pattern must have the scrutinee's type.
A range pattern has two literals of the same ordered scalar type; an exclusive
range's end must be greater than its start. A string is never a pattern. A
reference is followed structurally without an explicit dereference.

**S46. Coverage.** Every `match` is exhaustive (`E3032`) and has no unreachable
arm (`E3033`). Enums and `bool` are exhaustive once every constructor is
covered. Integer and character ranges partition their domain, so adjacent
ranges can cover it; floats need a catch-all. Slice patterns partition the
runtime length domain. A guarded arm never contributes coverage.

**S47. Pattern bindings.** Each name is bound once in a pattern (`E2040`); every
alternative of an or-pattern binds the same names with the same types (`E2041`).
A guard may read only `Copy` bindings, because a failed guard must leave the
scrutinee available for the next arm (`E2045`). Matching moves the scrutinee;
binding a non-`Copy` field moves that field out of the value.

**S48. Loop patterns.** A `for` pattern over a built-in range, array, or slice
MUST be irrefutable (`E3034`); a custom protocol loop may accept a refutable
pattern and skip non-matching items.

## `unsafe`, raw pointers, and C interop

**S49. Unsafe operations.** An `unsafe { ... }` block unlocks exactly: raw
dereference and projection, raw writes and copies, non-wrapping raw pointer
arithmetic and `offset_from`, access to `static mut`, and calls to `unsafe func`.
It does not disable type checking, initialization, moves, borrow checking, or
global effect analysis. An `unsafe func` is a caller contract; its body still
needs its own `unsafe` block for unchecked operations.

**S50. Raw pointer contracts.** A raw access requires a live allocation, an
in-bounds offset, proper alignment, and an initialized value of the pointee type.
`add`/`sub`/`offset` and friends are inbounds and must stay within the allocation
or one past its end; the `wrapping_*` family may compute outside it, but an
access there is undefined. `offset_from` requires one allocation. `copy` allows
overlap; `copy_nonoverlapping` requires disjoint regions. A write through a raw
pointer does not drop the old value. An integer-built pointer has no provenance
and may only be tested, compared, or stored; access through it is outside the
language contract.

**S51. C declarations.** An `extern "c"` function or static is called or read
only in `unsafe`. Its signature uses FFI-safe types: C scalars, `extern struct`,
raw pointers, and opaque `extern type`/`c_void` behind pointers. A Seal struct
without `extern`, a `bool` field, a zero-sized field, a reference, a slice, a
trait object, or a non-FFI-safe aggregate is rejected at the boundary
(`E3084`/`E3085`). An immutable external static is never written. C storage is
owned and released by C; no Seal drop or allocation is implied across the
boundary.

**S52. C layout.** `extern struct` keeps declaration order and the target C
size, alignment, and field offsets. A nested `extern struct` and an array of
C-safe elements are laid out recursively. `extern type` and `c_void` have no
size, alignment, fields, destructor, or by-value use; they appear only behind a
raw pointer (`E3087`). A C variadic call passes only FFI-safe arguments after
the fixed parameters.

## Constants, statics, and compile-time evaluation

**S53. Constants.** A `const` has no storage: reading it produces a fresh value,
projecting a field reads that part, and `&CONST` materializes a program-lifetime
allocation. A const is always initialized and never writable. Its initializer is
evaluated at compile time and cannot call non-`const` functions, use dynamic
dispatch or a custom protocol, carry `*mut T`, call an external function, read an
external static, or drop a value that needs a destructor. A value dependency
cycle is `E4007`.

**S54. Statics.** A `static` is one allocation for the whole program,
initialized before execution. An immutable static is read-only. A `static mut`
is writable only inside `unsafe`, and a reference into it may not escape the
function that created it (`E5012`). A block-scope static is one allocation per
declaration, shared by every call and monomorphization. A static is never
dropped. A non-`Copy` value cannot be moved out of a static.

**S55. Constant evaluation.** Evaluation is deterministic and bounded. It reads
only source, evaluated values, and the target data model; it never reads the
clock, environment, or host layout. Allocation order, dependency order, step
counting, and diagnostics are deterministic. A reference produced by constant
evaluation names a compiler-owned allocation with an in-allocation offset. An
unknown address (`E4030`), a write (`E4031`), or an invalid read (`E4032`) is
rejected. A reference to a const-frame allocation may not escape (`E4013`).

## Deprecation and lint control

**S56. Deprecation metadata.** `deprecated(since: "...", note: "...",
replacement: "...")` marks a function, constant, static, struct, enum, opaque
type, type alias, trait, trait method, inherent method, field, variant, or
extern function or static. `since` is a required semantic version; `note` and
`replacement` are
optional. The metadata is carried in the public API and documentation. A use of
a deprecated declaration warns with the `deprecated` lint at the use site;
uses inside the deprecated declaration itself do not warn. A `cfg`-inactive use
does not warn.

**S57. Lints.** `deprecated` and `unused_import` are individual lints. A lint
has `allow`, `warn`, or `deny` level. Unknown names are `E8112`; `warnings` is
the command-level group for every warning and is not itself a lint name.
`allow` suppresses, `warn` reports, and `deny` turns the diagnostic into an
error. Deny wins over allow when both are supplied for the same lint.

## Diagnostic contract

**S58. Stable codes.** Every rejection has a stable code in its phase range:
`E0xxx` lexical, `E1xxx` syntax, `E2xxx` name resolution and macros, `E3xxx`
type checking, `E4xxx` constant evaluation, `E5xxx` borrow checking, `E6xxx`
code generation, `E7xxx` monomorphization, `E8xxx` driver/project/lint, and
`E9xxx` standard library. Codes are never reused with a different meaning. A
warning is identified by lint name, not by code.

**S59. Spans.** A diagnostic points at the smallest source range that explains
the rejection. The primary span is inside the file being checked; secondary
spans may point at a declaration, an earlier use, or another file. A recovered
tree still has valid spans and IDs, but no recovered program is accepted.

**S60. No safe undefined behavior.** A safe program that satisfies S39–S48 and
S49–S55 has no undefined behavior. Undefined behavior can only be reached
through an unchecked operation whose documented contract is violated; the
contract is part of [`dynamic-semantics.md`](dynamic-semantics.md). A type,
initialization, move, or borrow error is never silently downgraded to a warning.

## Type aliases

**S61. Identity.** A declaration `type Name = Target;` introduces a name in the
type namespace but no new type identity. After substituting its arguments, a use
has the same type, layout, destructor, trait implementations, and ABI as the
written target. An alias never changes a value's ownership or evaluation order.

**S62. Declaration rules.** An alias is a file-scope item. Its name follows the
type-namespace uniqueness rules (S9): a same-name struct, enum, opaque type,
trait, or alias is `E2005` (or `E2083` across module fragments). `public`,
`internal`, `private`, `cfg`, and `deprecated(...)` have their ordinary meaning
(S6, S56). `test`, `bench`, `export`, and `unsafe` do not apply (`E1001`/
`E3071`). The declaration requires `=` and `;` (`E1002`).

**S63. Expansion and generics.** An alias may declare type parameters and is
used with exactly one type argument per parameter; a missing or extra argument
is `E2052`/`E2053`. Lowering substitutes the arguments into the target with the
same operation used for every generic declaration. The target may name a
primitive, tuple, reference, raw pointer, array, slice, function pointer, C
signature behind a raw pointer, trait object, struct, enum, opaque type behind a
raw pointer, type parameter, or another alias. A bound on an alias parameter or
a `where` clause on the alias is `E2096`: a transparent alias has no operations
that could enforce it.

**S64. Cycles.** An alias target must have a finite expansion. A reference that
reaches an alias already being expanded is `E2095`, including direct
(`type A = A;`) and mutual (`type A = B; type B = A;`) cycles. An alias that
fails expansion denotes the error type; later uses do not repeat the diagnostic.

**S65. Nominal paths and API.** In a type position a use always expands. In a
constructor, variant path, associated-function path, pattern, or impl header,
the expanded target is used as the nominal type: its fields, variants, methods,
and coherence rules apply. A target that is not a struct or enum in a
nominal-only position is `E2097`. A public alias is part of the package API and
its target must be nameable outside the package; otherwise it is `E3107`, and
the rejected alias does not enter the API snapshot, including through a public
re-export. A public re-export follows S5 and keeps the canonical declaration as
the API target.
