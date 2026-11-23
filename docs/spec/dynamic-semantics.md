# Dynamic semantics

This chapter defines what an accepted program does when it runs: evaluation
order, values, memory, layout, calls, destruction, panics, and the boundary of
defined behavior. It is the contract a backend must preserve at every
optimization level.

The model is deliberately small. There is no garbage collector, no reference
counting, no unwinding, no hidden allocation, and no threads in language `0.1`.
Every cost and every lifetime is visible in the source or in the static
obligations of [`static-semantics.md`](static-semantics.md).

## Values and evaluation

**D1. Value model.** A value is one of: a scalar (`bool`, `char`, integer,
float), a reference or raw pointer (thin or fat), an aggregate (struct, enum,
tuple, array), a closure environment, a function pointer, or unit. A place
denotes storage holding a value. Reading a place produces its value; writing a
place replaces it. A `Copy` read duplicates bits; a non-`Copy` read in a
move context transfers ownership and leaves the source uninitialized.

**D2. Left-to-right evaluation.** Except for the assignments below, operands are
evaluated left to right before the operation that consumes them. Function
arguments are evaluated left to right. Method calls evaluate the receiver first,
then the arguments left to right; the method body runs after all arguments are
evaluated. A struct literal evaluates fields in source order; an array literal
evaluates elements in source order. A tuple literal evaluates elements in source
order.

**D3. Assignment order.** A simple assignment evaluates the right-hand value
before the target place, so `grid[0] = grid[0] + 1` finishes the read before
`IndexMut` borrows the base. A compound assignment or `++`/`--` evaluates the
target place once first, reads the old value, then evaluates the right-hand side.
The place is not evaluated twice.

**D4. Short-circuiting.** `&&` evaluates its left operand; the right operand is
evaluated only when the left is `true`. `||` evaluates the right operand only
when the left is `false`. The result is always `bool`, and the skipped operand
has no effects.

**D5. Conditions and matches.** An `if` condition is evaluated once. A `match`
scrutinee is evaluated once, then patterns are tested in source order; the first
matching arm whose guard is true runs. An `if let` scrutinee is evaluated once.
A `while let` re-evaluates its scrutinee before every test. A `while` condition
is evaluated once per iteration. A `for` source is evaluated once; both range
bounds are evaluated once, before the first iteration.

**D6. Indexing and slicing.** An index expression is evaluated once and
bounds-checked before the access. A slice range evaluates its bounds once, in
source order, then checks `start <= end <= length`; a violation panics. A
constant out-of-range array index is a compile-time error, not a runtime panic.
An array or slice element access is a place and follows the ordinary read/write
rules.

**D7. Calls and returns.** A call evaluates the callee (unless it is a direct
function item), then the arguments, then enters the body. `return` evaluates its
value and exits the function, running the drops of every exited scope. Falling
off a function body returns its tail value; a body without a tail returns `()`.
A diverging expression (`return`, `break`, `continue`, `panic`, `exit`) does not
produce a value.

**D8. Closures.** Creating a closure captures the places named in its body in
first-mention order. A default capture stores a reference; `move` stores the
value (copying a `Copy` value, moving a non-`Copy` one). Calling a closure
evaluates its arguments and runs the lifted body with the environment as the
receiver. A closure with no captures has a zero-sized environment.

**D9. Integer arithmetic.** Signed integers are two's complement. `+`, `-`, `*`,
unary `-`, and compound arithmetic wrap modulo `2^N`; overflow is defined and
does not panic. Division and remainder panic on a zero divisor, and signed
`MIN / -1` panics. Shift counts are masked to the operand width (`count &
(N-1)`), so a shift by `N` is a shift by zero. Bitwise operators act on the
two's-complement bits.

**D10. Floating point.** `f32` and `f64` are IEEE 754 binary32 and binary64.
Arithmetic, comparisons, and conversions follow IEEE semantics, including NaN
and signed zero. An ordered comparison is false when either operand is NaN;
`!=` is true when either operand is NaN. Floating arithmetic does not trap.

**D11. Equality.** `==` and `!=` compare values, never addresses, and never move
their operands. The structural rules are the table in
[`../types.md`](../types.md#equality). `*str` compares length then bytes;
slices compare length then elements; structs compare fields in declaration
order; enums compare the active variant and its fields. References, trait
objects, function pointers, and closures are not comparable. A `NaN` is not
equal to itself; `-0.0 == 0.0` is true.

**D12. Casts.** A numeric cast reads its operand once and writes only the
destination. Integer narrowing keeps the low bits; widening sign-extends a
signed source and zero-extends an unsigned one. Float-to-integer truncates
toward zero, maps NaN to zero, and saturates at the destination bounds.
Integer-to-float rounds to nearest, ties to even. Float narrowing rounds to
nearest, ties to even, and overflows to infinity. A cast never allocates,
panics, or creates a loan.

## Memory

**D13. Storage classes.** Values live in locals (stack or an equivalent
backend-managed frame), in `static` storage with program lifetime, in
compiler-owned constant allocations, or in explicitly allocated heap storage.
Seal has no implicit heap allocation. The allocation primitives (`seal_alloc`,
`seal_dealloc`, and the standard-library wrappers) are ordinary calls with an
explicit ownership contract.

**D14. References.** A live `*T`/`*mut T` is non-null, properly aligned, and
points to initialized storage of its pointee type for as long as the loan is
live. A reference is a borrowed view, not an owner: copying or moving it does
not copy or destroy the pointee. `*mut T` grants exclusive access; `*T` grants
shared access. The static borrow checker is the proof of D14 for safe code.

**D15. Raw pointers and provenance.** A raw pointer carries an address and, for
a fat pointer, a metadata word. A pointer formed from a reference keeps that
allocation's provenance. `usize`/`isize` casts and `addr`/`expose_addr` observe
an address but do not create a usable provenance; an integer-built pointer may
be compared, tested for null, and stored, but an access through it is outside
the language contract. `with_addr` preserves metadata; `offset_from` requires
both pointers to belong to one allocation. Non-wrapping arithmetic is inbounds;
wrapping arithmetic may leave the allocation but an access through the result is
still constrained by D16.

**D16. Undefined behavior.** The following are undefined behavior when reached
through an unchecked operation in `unsafe` or across a foreign boundary:

- dereferencing or writing through a pointer that is null, misaligned, outside
  its allocation, or points to uninitialized or already-freed storage;
- reading or writing a value of the wrong type, or violating the aliasing
  contract of an exclusive reference;
- calling a function through a pointer with the wrong signature or ABI;
- violating a C function's documented ownership, lifetime, or aliasing contract;
- forming a reference that is not valid under D14, or letting a loan outlive its
  storage;
- accessing a `static mut` outside `unsafe`, or through a reference that escaped
  its allowed scope;
- executing a program whose safe static obligations were bypassed by an
  incorrect external declaration.

Undefined behavior is not a runtime error with a specified message. The
compiler may assume it does not occur when optimizing. Safe Seal code that
satisfies the static semantics cannot reach it.

**D17. Dropping.** When a scope exits normally, its locals are dropped in
reverse declaration order, followed by parameters. A struct runs its declared
destructor and then drops fields that need it in declaration order; an enum
drops only the active variant's fields; an array drops elements in increasing
index order. A moved value is not dropped at the source. `drop(value)` drops
immediately; `mem::forget(value)` consumes the value and runs no destructor, so
its resources leak. A `static` is never dropped. A panic does not unwind and
does not run the destructors of frames it abandons.

**D18. Overwrite.** Assigning to an initialized place whose type needs a
destructor drops the old value before storing the new one. A path-dependent drop
site is guarded by a flag that moves and drops clear and assignments set. A
definitely dead drop site is removed.

## Layout and ABI

**D19. Language layout.** Layout is exactly the algorithm in
[`../types.md`](../types.md#layout): natural alignment, declaration order, no
reordering, enum tag first, deterministic padding. Layout does not depend on the
host C compiler except for `extern struct`. A zero-sized value occupies no bytes
but has alignment 1; a `static` of zero-sized type still has a distinct address.

**D20. Internal ABI.** Seal function pointers and calls use the internal Seal
ABI, not the C ABI. A `func` value is one pointer-sized word; a closure is an
inline environment plus its lifted code. Passing a value follows the move/copy
rules, not C's by-value conventions. The internal ABI is stable for one compiler
version and is not an external contract.

**D21. C ABI.** An `extern "c"` declaration uses the target's C ABI. C scalars
map as in [`../types.md`](../types.md#c-scalar-types); `extern struct` uses C
layout and is classified according to the target ABI when passed by value. A
Seal struct without `extern`, a reference, a slice, a trait object, `bool` as a
field, or a zero-sized field is not C-safe. C function pointers are read-only
and always behind `*raw`. No `noalias`, `readonly`, or `nounwind` promise is
inferred for an external declaration.

**D22. Entry point and exit.** A hosted executable starts at the platform entry
and runs the language `main`. A freestanding target runs `_start` and has no
runtime. A language `main` takes no parameters and returns `i32` or `()`; a
missing or differently shaped entry point is `E6001`. A normal return from
`main` uses its integer value as the process exit status, or `0` for `()`.
`exit(code)` terminates immediately with `code`. `panic(message)` and a failed
bounds/division check terminate with status `101` on a hosted target and trap on
a freestanding target. No destructors run during termination, because Seal has
no unwinding.

## Constants and statics at run time

**D23. Static initialization.** Every `static` allocation exists before
execution and is initialized from its compile-time value, including
relocations. An immutable static is read-only after relocation. A zeroed
`static mut` may live in BSS. Reading a static produces its value or a
reference to its storage according to the place context; moving a non-`Copy`
value out of a static is rejected statically.

**D24. Constants.** A `const` has no storage of its own. Reading it produces the
evaluated value; `&CONST` materializes a program-lifetime allocation. A constant
value is the same at compile time and run time, including NaN and signed zero.
A constant cannot observe an address (`E4030`), write (`E4031`), read an invalid
raw location (`E4032`), call an external function, or read an external static.

**D25. Determinism.** For the same source, plan, target, and optimization
settings, the observable behavior and generated artifact are deterministic. The
compiler does not read the clock, environment, locale, host layout, or hash
iteration order when deciding language semantics. `seal verify-repro` checks two
independent builds byte for byte.

## Concurrency and effects

**D26. No threads in the language.** Language `0.1` has no thread creation,
atomics, or shared-memory synchronization primitives. A C callback may run on a
foreign thread, but borrowed Seal data must remain valid under the C contract;
Seal does not infer synchronization. When threads arrive, safe `static mut`
access must move behind atomics or locks, or gain a `Sync`-style contract.

**D27. Observable effects.** The observable effects of a program are its exit
status, bytes written through the standard streams and FFI, volatile external
state, and the sequence of panics. Allocation addresses and internal layout are
not observable in safe code. `expose_addr` is the explicit way to observe an
address; it is a raw-pointer operation and its result may differ between runs
and builds.

## Compile-time and run-time agreement

**D28. Same rules.** Constant evaluation implements the same arithmetic,
equality, layout, and drop rules as run time. A constant initializer and the
same expression at run time agree, except that an address observed at run time
may differ and constant evaluation rejects observations it cannot make
deterministically.

**D29. Optimization.** Every optimization level preserves D1–D28. `-O0` and
`-O2` may differ in timing and code size, never in language-visible results.
A `noalias`-style attribute is emitted only when the compiler has proved the
corresponding exclusivity; it is never a promise that unsafe code must uphold.

**D30. Type aliases are erased.** A type alias has no run-time representation.
After generic substitution, every operation on an alias is an operation on its
target: layout, field offsets, drop glue, dispatch, symbol names, and ABI are
the target's. An alias cannot change evaluation order, allocate, copy, or
introduce a runtime check.
