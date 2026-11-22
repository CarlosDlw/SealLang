# Raw pointers and `unsafe`

Status: implemented in the current language version.

A raw pointer is a nullable, copyable address with provenance. It is the
explicit escape hatch from Seal's checked references: the type tells you
whether the pointer may write, the value can be null or advanced, and every
operation that cannot be checked by the compiler must appear inside an
`unsafe { ... }` block.

This chapter is the complete guide to the feature. It covers the type system,
casts, every pointer method, fat pointers, pointer arithmetic, the unsafe
contract, constant evaluation, provenance, diagnostics, and the patterns that
keep unsafe code auditable. The normative type and memory-model decisions live
in [`../types.md`](../types.md) and [`../memory-model.md`](../memory-model.md).

```seal
func main() -> i32 {
  let mut value = 41;
  let pointer: *raw mut i32 = &mut value as *raw mut i32;

  unsafe {
    *pointer = *pointer + 1;
  }

  value
}
```

The program returns `42`. The pointer is created from an exclusive reference,
the dereference and write are inside the unsafe block, and the ordinary binding
is still owned and borrowed normally.

## 1. The mental model

Seal has two pointer families with deliberately different jobs.

| Property | `*T` / `*mut T` | `*raw T` / `*raw mut T` |
|----------|-----------------|--------------------------|
| Nullability | Never null | Null is representable |
| Borrow checking | A checked loan | No checked loan; provenance only |
| Arithmetic | None | Element and byte arithmetic |
| Address casts | None | `usize`/`isize` round trips, with restrictions |
| Equality | Only for `*str`/`*[T]` contents | Address equality, always available |
| Creation | `&place`, `&mut place` | Reference cast, raw-to-raw cast, integer cast |
| Unchecked access | Never | Only inside `unsafe` |
| Layout | One word, or two for fat pointees | One word, or two for fat pointees |
| `Copy` / `Sized` / Drop | Yes / yes / no | Yes / yes / no |

The safe reference model is unchanged. A raw pointer is a value, not an owner:
copying it copies only the address and metadata. It never keeps a pointee alive,
never extends a checked loan, and never drops anything.

A raw pointer has three pieces of information:

1. **Allocation provenance** — which allocation the address came from, when the
   compiler can know it.
2. **Byte offset** — where inside that allocation the pointer currently names.
3. **Metadata** — a length for slices and strings, or a vtable for trait
   objects. A thin pointer has no metadata word.

The compiler checks syntax, types, mutability, and the unsafe gate. It does not
prove that the address is non-null, aligned, initialized, in bounds, or still
alive. Those are the programmer's obligations, stated at the operation that
relies on them.

## 2. Type syntax and properties

```text
*raw T          read-only raw pointer to T
*raw mut T      writable raw pointer to T
```

`raw` is contextual: it is a keyword only immediately after `*`. A binding,
parameter, field, or function named `raw` remains an ordinary identifier.

```seal
func raw(raw: i32) -> i32 {
  raw
}

func keep(pointer: *raw i32, mutable: *raw mut i32) {
  // The two pointer kinds are distinct types.
}
```

Rules:

- `*raw T` and `*raw mut T` never unify. Assigning one to the other without a
  cast is a type mismatch (`E3001`).
- Raw pointers never unify with `*T` or `*mut T` without a cast.
- Both forms are `Copy`, `Sized`, need no destructor, and are valid in locals,
  parameters, return types, struct fields, enum variants, tuples, arrays, and
  generic arguments.
- The pointee may be unsized: `*raw str`, `*raw [T]`, and
  `*raw dyn Trait` are themselves sized pointer values.
- A raw pointer is not a reference. `&value` is not automatically a raw
  pointer; write the cast explicitly.

### Layout

| Type | Size on a 64-bit target | Alignment |
|------|------------------------:|----------:|
| `*raw T`, `*raw mut T` for sized `T` | 8 | 8 |
| `*raw str`, `*raw mut str` | 16 | 8 |
| `*raw [T]`, `*raw mut [T]` | 16 | 8 |
| `*raw dyn Trait`, `*raw mut dyn Trait` | 16 | 8 |

The pointer width follows the target, so a 32-bit target uses 4-byte words.
The two-word forms are `{ data, metadata }`: data first, then length or vtable.
The metadata is opaque to the language except through `len()` and `data()`.

### Why the pointer kind matters

The kind is a static permission:

- `*raw T` is read-only. Reads and address queries are allowed; writes,
  `copy`, and mutable borrows are not (`E3075`).
- `*raw mut T` permits reads and writes.
- A shared reference can become `*raw T`, but never `*raw mut T`. A mutable
  reference may become either kind.
- A raw-to-raw cast may discard mutability. It may also gain it when the source
  is already a raw pointer, but an ordinary shared reference cannot be promoted.

## 3. Creating a raw pointer

### From a reference

```seal
static LIMIT: i32 = 7;

const READ_ONLY: *raw i32 = &LIMIT as *raw i32;

func writable(value: *mut i32) -> *raw mut i32 {
  value as *raw mut i32
}
```

A reference cast keeps the same pointee and the same allocation. It records the
offset of a field or array element, so the pointer names that element rather
than the allocation root. It does not copy the pointee.

A `const` may hold a read-only pointer into a static. A writable pointer is
formed at runtime from a mutable place: constant evaluation cannot contain a
mutable reference (`E4012`) or write a static (`E4015`).

| Source | Target | Accepted | Note |
|--------|--------|:--------:|------|
| `*T` | `*raw T` | yes | Read-only pointer |
| `*mut T` | `*raw mut T` | yes | Keeps write permission |
| `*mut T` | `*raw T` | yes | Downgrades to read-only |
| `*T` | `*raw mut T` | no | `E3074`; permission cannot be invented |
| `*S` | `*raw dyn Trait` | yes | Requires `S: Trait` and an object-safe trait |

### Raw-to-raw casts

```seal
func reinterpret(pointer: *raw mut i32) -> *raw u8 {
  pointer as *raw u8
}
```

Raw-to-raw casts preserve the address and replace the pointee and kind. They
are useful for byte-level work and for adding or removing write permission.
The compiler still enforces metadata compatibility:

- A cast cannot change the pointer shape. A thin pointer becomes a fat slice
  pointer with `with_len` (inside `unsafe`), and a byte slice pointer becomes a
  string pointer with `as_str`; a fat pointer becomes thin with `data()`.
- A slice pointer may change its element type while keeping the length
  metadata.
- A raw pointer to a trait object may be reborrowed as another compatible trait
  object only where the type checker can prove the metadata shape.
- A raw pointer to a sized value can change to another sized pointee.

### From a pointer-sized integer

```seal
const NULL: *raw i32 = 0usize as *raw i32;

func rebuild(address: usize) -> *raw u8 {
  address as *raw u8
}
```

Only `usize` and `isize` convert to a thin raw pointer. `u32`, `u64`, `i128`,
floats, and other integer widths are rejected (`E3074`). An integer-built
pointer has no allocation provenance in the language model: it may be stored,
checked for null, compared, and cast back, but dereferencing it is valid only if
the address names a live, properly aligned, initialized value. For a pointer
that must carry provenance, start from a reference instead of an integer.

The inverse cast is equally explicit:

```seal
let address: usize = pointer as usize;
let signed: isize = pointer as isize;
```

`addr()` and `expose_addr()` return the same address observation without a cast.
`addr()` is the normal query; `expose_addr()` documents that the program may
reconstruct a pointer from the value. See [provenance](#8-provenance-and-the-unsafe-contract).

### Null pointers

`0usize as *raw T` creates a null thin pointer. `is_null()` checks the data
word and is safe:

```seal
let null: *raw i32 = 0usize as *raw i32;
if null.is_null() {
  // no access here
}
```

A null pointer is still a `Copy` value and can be passed, stored, and compared.
There is no `null` literal, and a fat pointer cannot be built from an integer.
The null pointer is the only supported invalid-address sentinel; it is not a
reference and cannot be dereferenced.

## 4. Reading and writing memory

### Dereference and projections

A raw dereference is a place. It supports the same projections as a reference
dereference:

```seal
struct Point {
  x: i32,
  y: i32,
}

func main() -> i32 {
  let point = Point { x: 40, y: 2 };
  let pointer = &point as *raw Point;

  unsafe {
    (*pointer).x + (*pointer).y
  }
}
```

A slice pointer can be indexed through a raw dereference; `data()` is only
required before pointer arithmetic:

```seal
let values = [10, 20, 30];
let slice: *raw [i32] = &values[..] as *raw [i32];

unsafe {
  let second = (*slice)[1]; // 20
}
```

Reading a non-`Copy` pointee through `*pointer` moves it out of the pointed-to
storage. The original place is left without a value; it is not dropped again.
Writing through `*raw mut T` stores the new value and does **not** run the old
value's destructor. Use an explicit safe operation or drop glue when ownership
must be respected. Raw memory operations are about bytes and values, not about
language ownership.

### The unsafe gate

Every unchecked memory access needs an `unsafe` block:

```seal
let pointer = &value as *raw i32;

let read = unsafe { *pointer };       // dereference
unsafe { *mutable = 42; }             // write through *raw mut T
```

Outside `unsafe`, the compiler reports `E3070`. The gate is lexical: the
operation must be inside an enclosing `unsafe { ... }` expression. A safe
function may contain an unsafe block; an `unsafe func` body does not become an
unsafe block automatically.

The unsafe block does not disable type checking, initialization, move checking,
borrow checking, or effect checking. It only unlocks the operations whose
preconditions the compiler cannot prove.

### Reads and writes as methods

`read`, `write`, and their unaligned forms are useful when the pointer is a
value and a place expression would be awkward:

```seal
let pointer: *raw mut i32 = &mut value as *raw mut i32;

unsafe {
  pointer.write(42);
  let value = pointer.read();
}
```

`read` requires `*raw T` or `*raw mut T` and returns `T`. `write` requires
`*raw mut T` and takes `T`. Both require a sized pointee. `read_unaligned` and
`write_unaligned` have the same types but set alignment 1, so they are valid for
a deliberately misaligned address. The unaligned forms do not make an invalid
address valid; they only relax alignment.

`copy` and `copy_nonoverlapping` copy whole values:

```seal
let source = [1, 2, 3];
let mut target = [0, 0, 0];

let src = &source[0] as *raw i32;
let dst = &mut target[0] as *raw mut i32;

unsafe {
  dst.copy(src, 3);              // overlap allowed: memmove
  dst.copy_nonoverlapping(src, 3); // regions must not overlap: memcpy
}
```

The count is a `usize` number of elements, not bytes. The copy size is
`count * size_of(T)`. The source may be a read-only raw pointer; the destination
must be `*raw mut T`. No destructor runs for overwritten values. A
zero-element copy is valid.

### Complete method table

`Same` means the receiver's pointer type, including kind and pointee. `T` is the
pointee. The **unsafe** column is the gate required at the call site.

| Method | Signature | Unsafe | Available on | Meaning |
|--------|-----------|:------:|--------------|---------|
| `is_null` | `(self) -> bool` | no | all | Whether the data address is zero |
| `addr` | `(self) -> usize` | no | all | The data address, without an explicit provenance observation |
| `expose_addr` | `(self) -> usize` | no | all | The data address, for deliberate reconstruction |
| `with_addr` | `(self, address: usize) -> Same` | no | all | Replace the data address, preserving metadata |
| `len` | `(self) -> usize` | no | `*raw str`, `*raw [T]` | Length metadata |
| `data` | `(self) -> *raw U` | no | all | Thin data pointer; `U` is the element, `u8`, or `()` |
| `with_len` | `(self, length: usize) -> *raw [U]` | yes | thin, sized pointee | Build a slice pointer from a data pointer and a length |
| `as_str` | `(self) -> *raw str` | yes | `*raw [u8]`, `*raw mut [u8]` | Reinterpret a byte slice pointer as a string pointer without validation |
| `add` | `(self, count: usize) -> Same` | yes | sized pointee | Forward by whole elements |
| `sub` | `(self, count: usize) -> Same` | yes | sized pointee | Backward by whole elements |
| `offset` | `(self, count: isize) -> Same` | yes | sized pointee | Forward or backward by signed element count |
| `byte_add` | `(self, count: usize) -> Same` | yes | sized pointee | Forward by bytes |
| `byte_offset` | `(self, count: isize) -> Same` | yes | sized pointee | Forward or backward by signed byte count |
| `wrapping_add` | `(self, count: usize) -> Same` | no | sized pointee | Element offset without the inbounds contract |
| `wrapping_sub` | `(self, count: usize) -> Same` | no | sized pointee | Element retreat without the inbounds contract |
| `wrapping_offset` | `(self, count: isize) -> Same` | no | sized pointee | Signed element offset without inbounds |
| `offset_from` | `(self, other: Same) -> isize` | yes | sized pointee | Element distance within one allocation |
| `read` | `(self) -> T` | yes | sized pointee | Load `T` with natural alignment |
| `write` | `(self, value: T) -> ()` | yes | `*raw mut T` | Store `T` with natural alignment |
| `read_unaligned` | `(self) -> T` | yes | sized pointee | Load `T` with alignment 1 |
| `write_unaligned` | `(self, value: T) -> ()` | yes | `*raw mut T` | Store `T` with alignment 1 |
| `copy` | `(self, source: *raw T, count: usize) -> ()` | yes | `*raw mut T` | Copy elements, allowing overlap |
| `copy_nonoverlapping` | `(self, source: *raw T, count: usize) -> ()` | yes | `*raw mut T` | Copy disjoint elements |

`data()` changes the pointee to the thin element type: for `*raw [i32]` it
returns `*raw i32`, for `*raw str` it returns `*raw u8`, and for
`*raw dyn Trait` it returns `*raw ()`. The pointer kind is preserved, so
`*raw mut [i32].data()` is `*raw mut i32`.

### Building a fat pointer from a thin one

C APIs commonly hand back a data pointer and a length separately. `with_len`
assembles the two words without exposing the address as an integer:

```seal
extern "c" { func read(fd: c_int, buffer: *raw mut u8, count: usize) -> isize; }

let mut buffer = [0u8; 64];
let base = &mut buffer[0] as *raw mut u8;
let count = unsafe { read(0, base, 64usize) };
let bytes: *raw mut [u8] = unsafe { base.with_len(count as usize) };
```

The receiver must be a thin pointer to a sized pointee; calling `with_len` on
`*raw [T]`, `*raw str`, or `*raw dyn Trait` is `E3075` because those already
carry metadata. The call is `unsafe`: the caller promises that `length` covers
live, initialized elements, that the range fits in the allocation, and that the
address is valid for that range. The data word keeps its provenance.

`as_str` changes the pointee of a byte slice pointer to `str`:

```seal
let text: *raw str = unsafe { bytes.as_str() };
```

The mutability is preserved: `*raw mut [u8].as_str()` is `*raw mut str`. Only
`[u8]` is accepted; `*raw [i32]` is `E3075`. The caller promises that the bytes
are valid UTF-8, because the string operations trust that invariant. The data
and length words are unchanged, so no allocation or copy occurs.

`as_str` is a **raw reinterpretation**, not a validator. It produces a raw
`*raw str` with no checked loan and no UTF-8 check. Safe code should use the
standard-library constructors instead:

```seal
let bytes: *[u8] = &data[..];

// Safe: validates the complete UTF-8 encoding. The returned view borrows bytes.
let checked: *str = match std::str::from_utf8(bytes) {
  Option::Some(text) => text,
  Option::None => return 1,
};

// Unsafe: only after the caller has already established UTF-8.
let unchecked: *str = unsafe { std::str::from_utf8_unchecked(bytes) };
```

`from_utf8` returns `Option<*str>` and keeps the checked loan on the byte
storage; invalid input returns `Option::None`. `from_utf8_unchecked` skips the
validation but still returns a checked `*str` view, so it is the normal unsafe
constructor for already validated bytes. The underlying compiler intrinsic is
`*[u8].as_str_unchecked()`: it is unsafe as well, and unlike raw `as_str` it
preserves the shared loan. Use raw `as_str` only when a raw string pointer is
actually required, such as at a low-level boundary; never use it to bypass
validation in safe code.

### Restoring a checked reference with `owned_view`

A raw pointer never carries a checked loan. When a container owns the
allocation and stores the address, it can turn that address back into an
ordinary reference with `owned_view`:

```seal
struct Buffer {
  pointer: *raw mut i32,
}

func shared(buffer: *Buffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer as *raw i32) }
}

func exclusive(buffer: *mut Buffer) -> *mut i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
```

The first argument is the **owner**: the storage that guarantees the allocation
stays alive. The second argument is the **source**: a raw pointer into that
allocation. The pointer kind selects the result, so `*raw P` produces `*P` and
`*raw mut P` produces `*mut P`. The conversion copies the data and metadata
words; it does not allocate, copy the pointee, validate the address, or rebuild
a fat pointer.

`owned_view` is an unsafe builtin (`E3070` outside an `unsafe` block) because it
restores a proof the compiler cannot derive from the pointer alone. The caller
must establish that:

1. `source` points into the allocation owned by `owner`;
2. the pointee is initialized and aligned for its type;
3. a slice or string length is valid, and string bytes are valid UTF-8;
4. the resulting reference kind matches the access the program will perform;
5. the owner outlives every use of the view.

The compiler enforces the part it can: `owner` must be an initialized checked
place, not a constant and not reached through a raw pointer. A local, a static,
a field, or a dereference of a checked reference are valid owners. A temporary
reference dereference such as `*identity(&buffer)` is also accepted: lowering
promotes the reference to the enclosing block, so the owner storage stays alive
for the view. A raw dereference such as `owned_view(*pointer, ...)` is rejected
with `E3106`.

The pointee may be a sized type, `[T]`, or `str`. Trait-object pointees are
reserved for a future extension and currently produce `E3106`. The primitive
is not available in a constant (`E4035`), because a constant cannot carry the
owner loan.

The reference behaves like any other reference after construction. Its loan is
on the owner, never on the raw pointer. While a shared view is live, the owner
cannot be moved, dropped, or written; an exclusive view additionally excludes
reads and other loans. Copying the view copies the loan, and passing or storing
it keeps the loan live through the copy. A view of a local owner cannot escape
(`E5004`); a view of a borrowed owner can be returned, in which case the
caller's loan is propagated. Because the body reaches storage through a raw
pointer, the function is excluded from `noalias`.

[`examples/053_owned_views.sl`](../../examples/053_owned_views.sl) is the
acceptance example for this section. It exercises thin, slice, and string
views, shared and exclusive owners, a promoted temporary owner, and a static
owner at `-O0` and `-O2`.

## 5. Arithmetic and bounds

### Element arithmetic

```seal
let values = [10, 20, 30, 40];
let base = &values[0] as *raw i32;

unsafe {
  let second = base.add(1);       // + sizeof(i32)
  let first = second.sub(1);      // - sizeof(i32)
  let third = base.offset(2);     // + 2 * sizeof(i32)
  let fourth = base.byte_add(12); // + 12 bytes
}
```

The amount is `usize` for `add`, `sub`, and `byte_add`, and `isize` for
`offset` and `byte_offset`. `add`, `sub`,
`offset`, `byte_add`, and `byte_offset` are the checked, inbounds family:
the result must remain inside the allocation or exactly at its one-past end.
Leaving that range is undefined behavior, even if the resulting address is
never dereferenced in a later operation that the compiler can see.

`wrapping_add`, `wrapping_sub`, and `wrapping_offset` are safe and may compute
an address outside the allocation. They do not make an access valid. The
wrapping family is for address computations that are intentionally later
checked or discarded, not for bypassing the bounds contract.

For a zero-sized pointee, every element offset has size zero, so
`add`, `sub`, `offset`, and `offset_from` preserve the address.

### `offset_from`

```seal
let values = [10, 20, 30];
let first = &values[0] as *raw i32;
let last = &values[2] as *raw i32;

unsafe {
  let distance = last.offset_from(first); // 2isize
}
```

Both pointers must belong to the same allocation and have the same element
type. Comparing pointers from different allocations is undefined behavior.
`offset_from` returns the signed number of elements; for a zero-sized pointee,
the distance is zero.

### Fat pointers and arithmetic

A `*raw str`, `*raw [T]`, or `*raw dyn Trait` carries metadata. Arithmetic on
the fat value is not available (`E3075`), because adding to the data word would
leave the length or vtable describing the wrong object. Call `data()` first:

```seal
let values = [10, 20, 30];
let slice: *raw [i32] = &values[..] as *raw [i32];

unsafe {
  let first = slice.data().read();
  let second = slice.data().add(1).read();
}
```

`data()` discards the metadata but keeps the pointer kind. `len()` remains
available on the original fat pointer. The metadata of a raw pointer is not
part of equality and is not changed by arithmetic, because arithmetic on a fat
pointer is rejected before lowering.

## 6. Address queries, equality, and reconstruction

### Safe queries

```seal
let value = 7;
let pointer = &value as *raw i32;

let null = pointer.is_null();       // false
let address = pointer.addr();       // data address
let observed = pointer.expose_addr();
let same = pointer.with_addr(address);
```

- `is_null()` is the supported null test. It is safe even for a fat pointer and
  checks only the data word.
- `addr()` observes the address for ordinary diagnostics and comparisons.
- `expose_addr()` states that the program may later reconstruct a pointer from
  the integer. It does not create provenance on its own.
- `with_addr(address)` replaces the data word and preserves the metadata word.
  The resulting pointer is only as valid as the address it was given.

### Equality

`==` and `!=` are available on raw pointers without `unsafe`. They compare the
data address only:

```seal
let same = first == first;          // true
let different = first == second;    // false for different allocations
let null = zero.is_null();          // separate null query
```

The pointee is not read, the value is not moved, and length/vtable metadata does
not participate. Ordering comparisons (`<`, `<=`, `>`, `>=`) are not available for raw pointers
(`E3002`).

## 7. Fat pointers: strings, slices, and trait objects

### Raw string pointers

```seal
let text: *raw str = "seal" as *raw str;
let length = text.len();              // 4
let first = unsafe { text.data().read() }; // 115u8, 's'
```

A string literal has type `*str`. Cast it to `*raw str` with `as`. `data()`
returns `*raw u8`; a byte read is a normal `u8`. The length is in bytes, not
characters. Raw string data is read-only; `*raw mut str` permits writes in the
type system, but writing into an interned string literal is still outside the
language contract because literals live in read-only storage.

`*mut str` has no safe method surface of its own. String methods are read-only
and reach the data through a `*str` reborrow; there is no safe operation that
writes through a mutable string view. Writing requires the raw methods and the
caller must preserve the UTF-8 invariant and initialization. Treat `*mut str`
as an ABI and `owned_view` target, not as a general writable string.

### Raw slice pointers

```seal
let values = [10, 20, 30];
let slice: *raw [i32] = &values[..] as *raw [i32];

let length = slice.len();             // 3
let first = unsafe { slice.data().read() }; // 10
```

The length is the number of elements. `data()` returns the element pointer.
A raw slice pointer does not carry a checked loan; it is the caller's job to
keep the underlying array alive and to stay within `0..length`.

### Raw trait-object pointers

```seal
trait Area {
  func area(self: *Self) -> i32;
}

struct Square { side: i32 }

impl Area for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }
}

func main() -> i32 {
  let square = Square { side: 6 };
  let object: *raw dyn Area = &square as *raw dyn Area;
  unsafe { object.data().read() } // data() is *raw (); not an Area call
}
```

A raw trait-object pointer is `{ data, vtable }` like its reference
counterpart. `data()` yields a thin `*raw ()` because the erased representation
has no useful pointee type; use it only as an opaque address. Dynamic calls
through the original `*raw dyn Trait` are not part of this surface; the pointer
is useful for carrying or inspecting the data address and metadata.

## 8. Provenance and the unsafe contract

### What provenance means for users

Provenance is the reason a pointer is not just an integer. A pointer derived
from `&value` knows the allocation that contains `value` and the offset it
names. A pointer derived from another raw pointer keeps that knowledge. An
integer cast loses it: the compiler may still represent the bits, but the
language no longer has an allocation proof for the address.

This is why the recommended path is:

```seal
let pointer = &value as *raw i32;
```

and not:

```seal
let address = 0x1234usize;
let pointer = address as *raw i32;
```

The second form is legal to write, but dereferencing it is the programmer's
responsibility and is not portable across runs.

The runtime aliasing model is **Tree Borrows** with pointer provenance. Every
allocation has a tree of permissions; a pointer carries a tag into that tree,
and an access is valid only while its permission permits it. Forming a pointer
does not by itself invalidate other pointers; incompatible accesses do. The
model is deliberately more permissive than Stacked Borrows for real unsafe
patterns, while still rejecting accesses outside the allocation's permission.

### The unsafe checklist

Before an unchecked operation, establish all of these:

1. The address is non-null, or the operation is a safe null query.
2. The address is aligned for `T`, unless the unaligned method is used.
3. The whole `T` lies inside a live allocation (or at a valid one-past position
   for arithmetic).
4. The memory is initialized for the read.
5. The access does not violate an exclusive loan that the program still relies
   on.
6. The allocation remains alive for the whole access.
7. For `offset_from`, both pointers are in the same allocation.
8. For `copy_nonoverlapping`, source and destination do not overlap.
9. For `write`, the destination came from a writable capability (`*raw mut T`).
10. For a value with ownership, the program does not drop or move it twice.
11. For `with_len`, the length covers live initialized elements in the
    allocation and the address is valid for that range.
12. For `as_str` and `from_utf8_unchecked`/`as_str_unchecked`, the bytes are
    valid UTF-8.

The `unsafe` block is where the compiler stops asking. It is not a place to
stop thinking; a short comment naming the invariant is part of good style.

### Aliasing and `noalias`

Safe exclusive references get `noalias` only when the compiler can prove the
whole program respects the exclusive grant. A raw pointer has no checked loan,
so a body that reads, writes, copies, or calls through raw storage is recorded
as possibly touching memory outside the safe view. The backend then withholds
`noalias` from parameters whose grant could be violated. This is a compiler
optimization decision, not a permission for safe code to alias: safe aliasing is
still rejected by the borrow checker.

The practical rule is simple: wrap the raw accesses in a small unsafe function,
document the invariant, and keep the raw pointer's lifetime and allocation
explicit. Safe callers should see an ordinary safe signature whenever the
contract can be hidden behind one.

## 9. `unsafe` functions and blocks

`unsafe` may prefix a free function, inherent method, associated function, or
trait method. Calling it requires an `unsafe` block at every call site:

```seal
unsafe func increment(pointer: *raw mut i32) {
  unsafe {
    *pointer = *pointer + 1;
  }
}

func main() -> i32 {
  let mut value = 41;
  unsafe { increment(&mut value as *raw mut i32); }
  value
}
```

Rules:

- An `unsafe func` declaration is a caller contract. Its body is still checked
  and still needs its own `unsafe { ... }` around unchecked operations.
- A call outside `unsafe` is `E3070`.
- A trait declaration and its implementation must agree on `unsafe`; a mismatch
  is a contract error (`E2024`).
- The entry point cannot be unsafe (`E3073`), and a destructor cannot be unsafe
  (`E3072`).
- `unsafe` does not apply to structs, enums, fields, constants, or type aliases
  (`E3071`), because those have no call boundary at which a caller could
  establish a precondition.
- Nested `unsafe` blocks are allowed. The innermost lexical block unlocks the
  operation; leaving it restores the ordinary checking context.

`static mut` is another unsafe-only surface. Reading, writing, borrowing, or
projecting it requires an `unsafe` block. The effect analysis still tracks which
functions may read or write it across calls, so an unsafe block does not hide a
conflict from the rest of the program.

## 10. Raw pointers in constants and statics

Constant evaluation supports raw pointers with allocation provenance. A `const`
may name a static, a promoted temporary, or another constant allocation; a
`static` may store a raw pointer as part of its memory image. Addresses become
relocations in the object file, so no constructor runs at startup.

```seal
static LIMIT: i32 = 7;
static TABLE: [i32; 2] = [1, 2];

const POINTER: *raw i32 = &LIMIT as *raw i32;
const SLICE: *raw [i32] = &TABLE[..] as *raw [i32];
const NULL: *raw i32 = 0usize as *raw i32;
const VALUE: i32 = unsafe { *POINTER };
```

| Constant operation | Result |
|--------------------|--------|
| `&STATIC as *raw T` | Accepted; relocation to the static allocation |
| `0usize as *raw T` | Accepted; null pointer |
| `1usize as *raw T` without dereference | Accepted; no allocation provenance |
| `*POINTER` where `POINTER` names a static | Accepted when aligned, in bounds, and initialized |
| `POINTER as usize` | `E4030` |
| `POINTER.addr()` / `POINTER.expose_addr()` | `E4030` |
| `POINTER.write(...)` / `copy` into a raw pointer | `E4031` |
| Dereference of null, misaligned, out-of-bounds, or uninitialized storage | `E4032` |

A `const func` may receive a raw pointer and read through it when the pointer
names a compiler-owned allocation. This makes it possible to factor constant
code without making addresses a compile-time constant:

```seal
const func read_limit(pointer: *raw i32) -> i32 {
  unsafe { *pointer }
}

const VALUE: i32 = read_limit(&LIMIT as *raw i32);
```

A `static mut` may store a raw pointer, but any access to the static itself is
still unsafe-gated. A constant cannot write through a raw pointer, cannot observe
a link-time address, and cannot read an arbitrary integer-built address.

## 11. Diagnostics and troubleshooting

| Code | Meaning | Typical fix |
|------|---------|-------------|
| `E3070` | An unsafe operation is outside `unsafe` | Wrap the operation or call in `unsafe { ... }` |
| `E3074` | Unsupported pointer cast | Check the cast matrix; use `data()` before changing fat/thin shape |
| `E3075` | Invalid raw pointer operation | Use `*raw mut T` for writes; call `data()` before arithmetic on a fat pointer; use `with_len`/`as_str` for the supported shape changes |
| `E3106` | Invalid `owned_view` owner or source | Pass an initialized checked owner and a raw pointer to a sized, slice, or string pointee |
| `E4030` | Address is not computable in a constant | Query nullness or move address observation to runtime |
| `E4031` | Constant writes through a raw pointer | Build the value in the initializer or write at runtime |
| `E4032` | Constant reads through an invalid pointer | Keep the access at runtime or read a known allocation |
| `E4035` | `owned_view` in a constant | Build the view in a runtime function |
| `E5004` | A view of a local owner escapes | Return the view through a borrowed owner, or keep it inside the function |

Common mistakes:

- **`cannot cast` on `&value as *raw mut T`** — a shared reference cannot gain
  write permission. Use `&mut value` or a raw-to-raw cast from a mutable raw
  pointer.
- **`p.add(1)` on `*raw [T]`** — fat pointers have no element arithmetic. Call
  `p.data().add(1)`.
- **`p.write(v)` on `*raw T`** — the pointer kind is read-only. Declare or cast
  to `*raw mut T`.
- **`p as usize` in a constant** — addresses are fixed by the linker. Use
  `is_null`, equality, or a runtime function.
- **`*p` outside `unsafe`** — the dereference is an unchecked operation; move
  it into an unsafe block and document why the address is valid.
- **A pointer to a local escapes** — the pointer does not extend the local's
  lifetime. Keep the allocation alive, or use a `static`/caller-owned value.

## 12. Patterns that stay auditable

### Wrap the contract in one function

Prefer a small safe API over raw pointers spread through a program:

```seal
func swap_values(left: *raw mut i32, right: *raw mut i32) {
  unsafe {
    let temporary = left.read();
    left.write(right.read());
    right.write(temporary);
  }
}
```

The safe caller passes references cast at the boundary. The unsafe block is
short, the address assumptions are local, and the function signature documents
that both pointers must be valid and non-overlapping.

### Keep arithmetic in element units

Use `add`/`offset` for typed work and `byte_add` only for byte-oriented
protocols. This keeps the size multiplication visible in the type and avoids
mixing bytes and elements.

### Call `data()` once

For a slice or string, keep the fat pointer for `len()` and derive the thin
pointer once:

```seal
let slice: *raw [i32] = &values[..] as *raw [i32];
let length = slice.len();
let base = slice.data();
```

Prefer keeping the original fat pointer. When a C API returns a data pointer and
a length separately, use `with_len` (and `as_str` for bytes) rather than forging
the metadata with a cast; the compiler owns the layout of the length word.

### Prefer references until you need the escape hatch

Use `*T`/`*mut T` for ordinary APIs. Reach for raw pointers when the program
needs null, address arithmetic, an integer round trip, or an interop boundary
that the safe model cannot express. The safe model remains the default, and
unsafe code should be the smallest possible island.

## 13. Comparison with Rust

| Seal | Rust analogue | Difference |
|------|---------------|------------|
| `*raw T` | `*const T` | Seal's pointer is explicitly nullable and unsafe to dereference |
| `*raw mut T` | `*mut T` | The kind is part of the type and is checked for writes |
| `addr()` | `addr()` | Safe address observation |
| `expose_addr()` | `expose_provenance()` | Documents deliberate reconstruction |
| `with_addr()` | `with_addr()` | Replaces the data word; metadata is preserved |
| `read`/`write` | `read`/`write` | Same unsafe contract; no `MaybeUninit` layer yet |
| `copy`/`copy_nonoverlapping` | `copy`/`copy_nonoverlapping` | Same overlap split, memmove/memcpy lowering |
| `*raw dyn Trait` | `*const dyn Trait` | Two-word data/vtable representation |

Seal does not have `NonNull` or `MaybeUninit`, and it has no owning typed
wrappers such as `Box`, `Vec`, or `CString`. The hosted standard library does
expose an explicit allocator as `std::alloc`: validated `Layout` values,
`AllocError`, unsafe `alloc`/`alloc_zeroed`/`realloc`/`dealloc`, and the
non-`Copy` `RawBuffer` owner of one untyped block. Those APIs do not make raw
dereferences safe; every access through `RawBuffer::as_ptr` still requires
`unsafe`. Allocation remains hosted-only and absent from `core`, `--no-stdlib`,
and freestanding targets, and no C memory may be passed to the Seal
deallocator. See [`stdlib.md`](stdlib.md#allocation) for the public
surface. C interop, including C function pointers (`*raw extern "c" func(...)`)
and `c"..."` literals, is covered in [C interoperability](ffi.md); those are
distinct surface forms, not alternate spellings of the raw pointer operations
in this chapter.

### Growth path: typed containers

`owned_view` is the compiler-side growth hook for typed containers. A future
`Box<T>`, `Vec<T>`, and `String` can store a raw pointer and expose `as_ref`,
`as_slice`, and `as_str` as safe methods whose returned reference borrows the
container. Each wrapper calls `owned_view` internally and keeps its allocation
or UTF-8 invariant private, so safe callers never reconstruct a view themselves.
The same MIR rvalue and loan model can support an owned view of `dyn Trait` when
that arm is admitted, without changing the representation of references or
adding a new unchecked cast to safe code.

Keeping the conversion explicit also keeps the cost visible: no container is
required for a program to use `owned_view`, and adding one does not change the
allocation model, ABI, or runtime.

## 14. Complete example

[`examples/043_raw_pointers.sl`](../../examples/043_raw_pointers.sl) is the
acceptance example for this chapter. It creates an exclusive pointer into a
local array, advances it, writes through it, copies overlapping elements, checks
a null pointer, and returns `0` only when the address and values are correct.
It is compiled at `-O0` and `-O2` by the codegen suite.

```seal
func main() -> i32 {
  let mut values = [10, 20, 30, 40];
  let base = &mut values[0] as *raw mut i32;

  unsafe {
    let second = base.add(1);
    *second = *second + 2;

    let last = &mut values[2] as *raw mut i32;
    last.copy(base as *raw i32, 2);

    let distance = second.offset_from(base);
    let null: *raw i32 = 0usize as *raw i32;
    let total = *base + *second + *last;

    if null.is_null() && distance == 1 && total == 42 {
      0
    } else {
      1
    }
  }
}
```

The key facts:

- `base` is `*raw mut i32`, so the cast from `&mut values[0]` is allowed.
- `add(1)` is in bounds and remains in the unsafe block.
- `copy` uses overlapping `memmove` semantics.
- `offset_from` compares two pointers in the same allocation.
- `null` is an integer-built pointer used only for `is_null`.
- The safe binding `values` is not read again while the raw pointers are live.

## 15. Quick reference

```text
Type          *raw T              *raw mut T
Read          yes                 yes
Write         no                  yes
Nullable      yes                 yes
Copy/Sized    yes                 yes
Checked ops   unsafe              unsafe
Wrapping ops  safe                safe
Comparison    address equality    address equality
```

Accepted casts:

- `&place as *raw T`
- `&mut place as *raw mut T` and `&mut place as *raw T`
- `*raw T as *raw U` and `*raw mut T as *raw U`
- `*raw T as usize` / `*raw T as isize`
- `usize as *raw T` / `isize as *raw T` for a thin `T`
- `*raw [T] as *raw [U]` for compatible metadata

Rejected casts:

- `&place as *raw mut T` (`E3074`)
- `*raw T as u64`, `*raw T as f64`, `*raw T as *raw [U]`
- `*raw [T] as *raw U`
- `usize as *raw [T]` and `usize as *raw dyn Trait`
- any raw pointer cast that would forge metadata
