# Seal code generation

`seal-codegen` is the only crate that turns checked MIR into machine code. It
produces one artifact per invocation:

- a relocatable object (`sealc build --emit=obj`),
- target assembly (`sealc build --emit=asm`), or
- a linked native executable (`sealc build --emit=exe`, the default).

`sealc check` stops after monomorphization and writes nothing. Object and
assembly output do not require an entry point; executables do.

## Pipeline

1. **Declare.** Every instance in the monomorphized program becomes an LLVM
   function with external linkage and the signature derived from its concrete
   argument and return types. A non-generic free function is `seal.<name>`; a
   non-generic impl function is `seal.<Type>.<name>`, so same-named methods on
   different types stay distinct. A generic instance appends the stable
   fingerprint of its definition and full type arguments
   (`seal.identity.<hash>`), so two instantiations cannot collide and repeated
   builds produce the same symbol. Declaring first lets a body call a function
   declared later; calls resolve by `Instance`, never by name. Every collected
   vtable becomes one private unnamed constant global before any body is
   lowered.
2. **Lower.** Each body gets one stack slot per local, one LLVM block per MIR
   block, and explicit loads/stores for places. Struct field and tuple element
   projections use struct GEPs; array indices use a byte offset after the MIR
   bounds check; dereferences load the reference and continue from its pointee.
   Constant evaluation runs before borrow checking and yields a
   `ConstProgram`. Monomorphization adds the reachable statics and const
   allocations to `MonoProgram` in discovery order, including allocations
   reached only through another static and vtables named by constant values.
   Code generation reads those lists instead of a source initializer: a `const`
   use is materialized as its value, a static is one global initialized from its
   allocation, and a promoted temporary or addressed const becomes a private
   constant global.
3. **Debug information.** When requested, a `DISubprogram`, parameter and binding
   variables, aggregate types, and a `DILocation` per statement and terminator
   are emitted alongside the IR.
4. **Verify.** `Module::verify` runs before optimization. Invalid IR is an
   internal compiler error, never an artifact.
5. **Optimize.** `-O1` through `-O3` run LLVM's `default<On>` pipeline; `-O0`
   skips it. The target machine's optimization level matches the flag.
6. **Emit.** Object and assembly bytes come from LLVM's target machine and are
   written atomically. An executable is linked from a temporary object with the
   system C toolchain.

## Command line

```text
sealc build [OPTIONS] <INPUT>...
sealc check [OPTIONS] <INPUT>...
sealc run [OPTIONS] <INPUT> [-- <ARGS>...]
sealc emit-runtime <PATH>
sealc explain <CODE>
```

`sealc help` lists the commands, and `sealc help build` lists every build option
grouped by concern: output, optimization, target, diagnostics, and runtime.
Options accept `--name value` and `--name=value`. In `build` and `check`, `--`
ends option parsing so a file named like a flag stays an input; in `run`, `--`
separates the program arguments.

- `sealc build --emit=exe` (the default) links an executable,
  `sealc build --emit=obj` writes an object, and `sealc build --emit=asm` writes
  assembly. `sealc check` stops after monomorphization and writes nothing.
- Without `-o`, `sealc build` writes `file` for an executable, `file.o` for an
  object, or `file.s` for assembly, next to the input.
- `-o` accepts one output path and is rejected with more than one input.
- `-O` alone selects level 3; `-O0` through `-O3` and `--opt-level <LEVEL>`
  select a level explicitly.
- `-g` embeds DWARF in objects and executables; assembly contains the matching
  `.file`/`.loc` directives.
- `sealc emit-runtime <PATH>` writes the embedded runtime archive without
  compiling, and `sealc build --runtime-archive <PATH>` links against a
  different archive.
- `sealc explain <CODE>` prints an error-code explanation without compiling.
- `sealc run` builds an executable inside an isolated temporary directory, runs
  it with inherited standard streams, removes the directory on every path, and
  returns the program's exit status. It shares the optimization, runtime,
  target, and diagnostic options with `build`, but owns its output path. Only
  host programs can run: a target with another architecture or OS is rejected
  before compilation. Arguments after `--` are forwarded to the program.

The exit status is 0 on success, 1 after a compile error, 2 after a usage
error, and 101 after an internal compiler error. `sealc run` returns the
program's status, including 101 for a Seal runtime panic.

The linker is selected from `SEAL_LINKER`, then `CC`, then `cc`. Arguments are
passed directly, never through a shell. A linker failure is `E6003` and includes
the linker's stderr.

## ABI and layout

The ABI is defined by the language, not by a C header:

- A non-generic free Seal function has external visibility and the symbol
  `seal.<name>`; a non-generic impl function, inherent or from a trait
  implementation, uses `seal.<Type>.<name>`. A generic instance keeps that base
  and appends `.<hash>`, where the hash is the `seal-hash` stable fingerprint of
  the definition and every type argument, flattened outermost first. Symbols
  never contain type syntax, spaces, or angle brackets. The C `main` wrapper is
  the only symbol named `main`, and only a non-generic free function can be the
  language entry point.
- A user `main` must take no parameters and return `i32` or `()`. The wrapper
  always returns `i32`, using the returned value or zero for unit. A missing or
  invalid entry point is `E6001`.
- Locals, parameters, and the return place live in `alloca` slots. Reading and
  writing a place is a load or store; moving is a bitwise transfer because no
  current type has a destructor.
- Structs and tuples are LLVM structs with fields or elements in declaration
  order, natural alignment, and no reordering. Arrays are LLVM arrays of their
  element type: the element size already includes its trailing padding, so the
  array stride matches `seal-layout`. Scalars use their size as alignment,
  references use the target pointer width, `bool` is one byte, and `()`/`!` are
  zero-sized structs. Tuple literals lower to `Rvalue::Tuple`, which inserts each
  evaluated element into its declared position, and `Projection::TupleField`
  selects storage with a struct GEP. The layout comes from `seal-layout`, so
  code generation and semantic layout cannot disagree.
- An enum is an LLVM struct `{ tag, anchor, payload }`: the tag is the smallest
  integer that addresses every variant, a zero-length array of payload-aligned
  integers contributes the payload alignment without occupying storage, and the
  payload is a byte array as large as the largest variant. Constructing a
  variant stores the discriminant and then stores a variant-shaped struct at the
  payload address; the temporary lives in the function prologue, so a loop does
  not grow the stack. Every variant shares the same LLVM type, and positional
  and named fields are stored in declaration order. `match` reads the payload
  through the same offsets: a field pattern is a `Projection::VariantField` GEP
  into the payload, valid because the tag test selected that variant.
- A generic aggregate has no single LLVM type. `TypeMapper` caches one struct
  per `(AdtId, SubstsId)` pair: it reserves an opaque struct on first use, then
  fills it from the field types with the instance's arguments substituted, so a
  recursive reference finds the reservation instead of recursing forever.
  Field and variant GEPs, enum tag/payload offsets, and aggregate construction
  all query that same instantiation. Non-generic aggregates keep the historical
  `seal.adt.<index>.<name>` type name; each generic instantiation receives a
  unique suffix.
- `match` lowers to control flow. The scrutinee is moved into a temporary,
  `Rvalue::Discriminant` reads the tag into a `u32`, and a chain of `SwitchInt`
  tests selects the arm; a literal pattern compares with `Rvalue::Binary`
  `Eq` and switches on the boolean result, a range pattern compares twice (`Ge`
  on the start and `Le` or `Lt` on the end), a struct pattern projects each
  field with `Projection::Field`, a tuple pattern projects each element with
  `Projection::TupleField`, and an array pattern projects each fixed element
  with `Projection::Index` at a constant position; the patterns after `..` use
  positions measured from the array length. A slice pattern reads the
  runtime length with `Rvalue::Len`, switches on it (equality without `..`, a
  `>=` test with `..`), then indexes the dereferenced slice; suffix positions
  are computed as `length - k`, and the length test proves every index in
  bounds. A struct or variant pattern applied to a reference first dereferences
  the scrutinee place, so its tag read and field projections go through the
  pointer. An or-pattern becomes one
  such chain per alternative. Bindings are stores from the field or element
  places, emitted after the whole pattern matched, and a guard is another
  `SwitchInt` on its boolean result. Falling past the last arm enters an
  `Unreachable` block, which exhaustiveness checking already made impossible.
- Array indexing evaluates the index once, checks it against the length, and
  then computes `base + index * sizeof(element)`. Inkwell exposes GEP only as an
  unsafe operation and every compiler crate except the debug bridge forbids
  `unsafe`, so the address is formed with `ptrtoint`, integer addition, and
  `inttoptr` after the bounds check. The result addresses the same element and
  keeps the workspace's safety policy intact.
- A slice reference is the fat pointer `{ data, length }`. A range borrow checks
  `start <= end` and `end <= source length` in MIR, then forms the data address
  from the source and stores `end - start` as the length. A full `[..]` range
  performs no check. Slicing a slice reloads its fat pointer and applies the
  same arithmetic, so reborrows do not allocate.
- Custom indexing is an ordinary protocol call followed by a dereference.
  `grid[key]` reads through `Index::index` (or a vtable slot on `*dyn Index`),
  and a write or compound assignment reads and writes through
  `IndexMut::index_mut`; the returned reference is the place being accessed, so
  code generation emits the same call, load, and store shapes as any other
  pointer-returning method. A simple assignment lowers its value before the
  target place, so a read of the same container ends before the target acquires
  its exclusive borrow; compound assignments lower the target place first. The
  compiler adds no bounds check for a custom implementation; built-in arrays
  and slices keep the checked path above.
- A custom `for` loop is a small CFG. Lowering evaluates the source once, calls
  `IntoIterator::iter` when the source is not its own iterator, tests
  `Iterator::has_next`, runs the body with the item moved into the pattern, and
  calls `Iterator::next` on the backedge; `continue` goes to the condition and
  `break` to the exit. A failed refutable pattern skips the body without
  binding. The calls are direct, symbolic, or dynamic exactly like any other
  method call, so the loop allocates nothing and adds no new MIR terminator.
- User methods lower from `Rvalue::Call`; the receiver is the first argument.
  An automatic shared or exclusive receiver borrow is an explicit
  `Rvalue::Ref` in MIR, so code generation emits the ordinary address and load
  for the call. Associated functions are ordinary calls reached through a
  resolved function path. A static trait method takes the same path, and a
  `*mut T` receiver passed to a shared method becomes a shared `Rvalue::Ref` of
  the pointee.
- A trait-object pointer is the fat pointer `{ data, vtable }`. `MakeTraitObject`
  stores the concrete data address and the address of that pair's vtable
  constant; `TraitObjectReborrow` copies the two words and records the new loan
  without changing either. The vtable starts with a no-op drop function, size,
  and alignment, followed by method pointers in the shared `seal-layout` order:
  principal-trait methods in declaration order, then supertraits depth-first,
  deduplicated by method identity. A slot for a method whose impl omits it
  points at the trait's default-body instance, resolved by the same
  monomorphization fallback as a static call; the vtable layout does not change.
  `DynamicCall` loads the recorded slot from the receiver's own vtable, extracts
  the data word, and emits an indirect call
  whose first argument is that data pointer. It never inspects the concrete type
  or searches for a vtable at run time. Construction and dispatch allocate nothing and copy no concrete
  value. The vtable layout is compiler-private; Seal does not promise a stable
  external ABI for it.
- Intrinsic methods lower from `Rvalue::Intrinsic`. Scalar methods use LLVM
  instructions and target intrinsics: `llvm.abs`, `llvm.ctpop`, `llvm.ctlz`,
  `llvm.cttz`, `llvm.fshl`/`llvm.fshr`, `llvm.bswap`, `llvm.fabs`,
  `llvm.sqrt`, and `llvm.powi`, with explicit comparisons and `select` for
  min/max/clamp, signum, bit tests, character classes, and NaN-aware float
  selection. Saturating arithmetic uses `llvm.{s,u}{add,sub,mul}.with.overflow`
  and selects the type's bound on overflow; wrapping arithmetic uses plain
  integer instructions. Float rounding is implemented without libm: `trunc`
  converts to an integer and back with guards for NaN, infinity, signed zero,
  and values too large for the integer, and `floor`/`ceil`/`round` adjust that
  result by one when needed. `fract` subtracts the truncated value.
- Sequence methods split an array or slice receiver into its elements pointer and
  runtime length. `len` and `is_empty` use the length directly. `swap` checks
  both indices before either load, then exchanges elements through temporary
  storage sized for the element type. `reverse` walks two indices toward each
  other; an empty sequence starts both at zero so the loop does not run.
  `fill` loads the value once and stores it into every element. None of these
  loops allocates, and their storage lives in the function entry block.
- `clamp` checks `minimum <= maximum` at run time and calls `seal_panic` with a
  contract message when the ordering fails; `swap` reports an out-of-range
  index through the same path. A panic exits with status 101 and names the call
  site.
- `[value; N]` evaluates `value` once and fills a temporary array in a loop. The
  temporary is allocated in the entry block, so a repeat inside a loop does not
  grow the stack on every iteration.
- `*str` is the fat pointer `{ data, length }`. A literal becomes a private,
  unnamed constant array plus its byte length; no run-time terminator scan is
  needed.
- A `*T` or `*mut T` is one pointer-sized word: both kinds share a
  representation and differ only in the checks they enable, so the kind costs
  nothing at run time. The backend marks an exclusive-reference parameter
  `noalias` only when borrow checking proved the whole program upholds its
  exclusive grant: the function body never reaches storage through a reference
  of unknown target or a dynamic call, and every call site proves its reference
  arguments name disjoint storage. The proof covers every call site in the
  compiled program; calling an exported function from foreign code with aliasing
  pointers is outside it. A shared `*T` parameter is `readonly`: the type system
  forbids writing through it, so that promise needs no whole-program proof. A
  function reachable through a vtable is always excluded from `noalias`, because
  its indirect call sites never passed the static proof. A `*str`, `*[T]`,
  `*mut [T]`, `*dyn Trait`, or `*mut dyn Trait` parameter is a fat pointer,
  which LLVM cannot annotate, so it keeps the ordinary ABI.
- The object is position independent (`RelocMode::PIC`) and uses the target's
  default code model.

## Debug information

`-g` requests `DWARFEmissionKind::Full` with `DWARFSourceLanguage::Rust`:

- one compile unit per input file, with the displayed (possibly remapped) path;
- one `DW_TAG_subprogram` per monomorphized instance, named with readable type
  arguments (`identity<i32>`) and linked to the instance's LLVM function;
- `DW_TAG_formal_parameter` for parameters and `DW_TAG_variable` for bindings,
  declared at their stack slots with source line and column;
- `DW_TAG_structure_type` for every struct instantiation, named with its type
  arguments (`Pair<i32, bool>`), with instantiated member types, member names,
  bit offsets, and sizes from `seal-layout`; a tuple uses the same structure
  type with member names `0`, `1`, and so on;
- `DW_TAG_structure_type` for every enum instantiation with its discriminant
  member and a payload byte-array member at the layout offset. Variant parts are not emitted
  yet, so a debugger shows the tag and raw payload rather than the active
  variant's fields; the layout is still exact;
- pointer and fat-pointer types, including `*str`, `*[T]`, and `*dyn Trait` as
  two-member structs with data and length or vtable metadata;
- array types with their element type, length, size, and alignment;
- a line-table entry for every statement and terminator, so debuggers stop on
  source lines rather than function boundaries.

Recursive structs and enums use forward declarations for the pointee of a
reference; an aggregate cannot contain itself by value, so this models every
legal recursion.

LLVM 19 replaced debug intrinsics with debug records. The C API that appends a
declaration returns a record, not an instruction, and inkwell 0.11 mis-wraps
that return value. `seal-llvm-debug` is a deliberately tiny crate that calls the
record API directly and discards the returned handle. It is the only crate that
permits `unsafe`; every other compiler crate keeps `unsafe_code = "forbid"`.

## Defined runtime behavior

Code generation must not leave undefined behavior on ordinary arithmetic:

- `/` and `%` panic through the runtime when the divisor is zero, and for signed
  integers when the dividend is the minimum and the divisor is `-1`. LLVM treats
  both cases as undefined; Seal makes them deterministic runtime failures.
  Division and remainder use distinct messages for a zero divisor and for
  overflow.
- Shifts mask the amount to `width - 1` bits, so an oversized shift has a defined
  result instead of target-dependent undefined behavior.
- Integer add, subtract, and multiply wrap in two's complement. No operation
  inserts an overflow check.
- Float arithmetic and comparisons use IEEE semantics. Ordered comparisons
  (`OEQ`, `OLT`, ...) make a comparison with NaN false, and `!=` uses `UNE`, so
  `NaN != NaN` is true.
- A runtime array index is checked against the array length before the element
  address is formed. An out-of-range index calls `seal_panic` with
  `index out of bounds: the length is N` and exits with status 101. A slice
  index checks against the runtime length, and a slice range checks its bounds
  before constructing the fat pointer.
- Bitwise operators require integer operands (or `bool` for `!`), which the type
  checker enforces before code generation.

### Numeric casts

An explicit cast lowers from `Rvalue::Cast` and never re-infers the conversion:

- Integer to integer uses `trunc` when narrowing. A same-width cast is a no-op;
  widening uses `sext` for a signed source and `zext` for an unsigned one.
- Integer to float uses `sitofp` for a signed source and `uitofp` for an
  unsigned one. Rounding is LLVM's round-to-nearest, ties-to-even.
- Float to float uses `fpext` or `fptrunc`; the result follows IEEE rounding,
  signed zero, NaN, and overflow to infinity.
- Float to integer calls `llvm.fptosi.sat` or `llvm.fptoui.sat`. The saturating
  intrinsic truncates toward zero, maps NaN to zero, and clamps at both bounds.
  Plain `fptosi`/`fptoui` is deliberately never emitted, because it is undefined
  for out-of-range values, NaN, and infinity.

A cast emits no runtime call, allocation, temporary owner, or metadata. It reads
its operand once and stores one destination value, and debug locations still
describe the surrounding statement.

## Native runtime

`build.rs` compiles `runtime/seal_rt.c` with the `cc` crate and embeds the
resulting archive in the compiler. The runtime is minimal C11 with no
allocation, locks, stdio, locale, or `atexit` handlers:

- `seal_write` writes exactly the requested bytes to a descriptor, retrying
  `EINTR` and partial writes. `seal_print` and `seal_eprint` select stdout and
  stderr; on Windows they map the descriptor to `STD_OUTPUT_HANDLE` or
  `STD_ERROR_HANDLE` and use `WriteFile`.
- `seal_panic` is `noreturn`, `nounwind`, and `cold`. It writes
  `panic: <message> at <file>:<line>:<column>\n` to standard error and
  terminates with status 101, without flushing or unwinding. The location is
  the span of the failing expression, not the enclosing statement. Generated
  calls are followed by `unreachable`.
- The archive is linked after the generated object and only into executables;
  unused members do not enter the output. Object and assembly emission never
  need the runtime.
- `sealc emit-runtime <PATH>` writes the embedded archive for manual links of
  object files produced with `sealc build --emit=obj`.
- The archive is built for the compiler host. `sealc build --emit=exe` for
  another architecture or OS is rejected with `E6003` unless
  `--runtime-archive <PATH>` supplies an archive built for that target.
  `sealc build --emit=obj` remains available for cross-compilation.

## Linking

Executables use the linker selected by `SEAL_LINKER`, then `CC`, then the
platform default (`cl` on Windows, `cc` elsewhere). The argument style follows
the program name: `cl` and `clang-cl` use `/Fe:`, the standalone `link` uses
`/OUT:`, and every other toolchain uses `-o`. On Windows the driver appends
`.exe` to executable output paths that have no extension.

The linker writes to a temporary path in the output directory, which replaces
the destination only after a successful link. A failed link leaves an existing
executable from an earlier build untouched and never leaves a truncated one.

## Failures

| Code | Meaning |
|------|---------|
| `E6001` | The executable has no usable `main`, or `main` has parameters or a return type other than `i32`/`()`. |
| `E6002` | LLVM has no backend for the requested triple or cannot create a target machine. |
| `E6003` | The artifact could not be written, the linker could not run, linking failed, an external runtime archive could not be read, or an executable was requested for a target that does not match the embedded runtime and no replacement was supplied. |

Broken compiler invariants (invalid MIR reaching codegen, an unresolved inference
type, invalid generated IR, or a failed optimization pipeline) are internal
compiler errors, not user diagnostics.

## Extension points

- **New type kind.** Extend `TypeMapper::basic`, the debug type builder, and the
  constant/value paths that consume it. Keep `LayoutCtxt` as the single source of
  size and alignment.
- **New MIR rvalue or operand.** Add an arm to `FunctionLowerer::emit_rvalue` (or
  `load_operand`); places, debug locations, and verification stay unchanged.
- **New target.** `seal-target` supplies the triple, pointer width, and data
  model; LLVM supplies the backend. Cross-linking needs a matching linker,
  sysroot, and runtime archive, while `sealc build --emit=obj` works without
  them.
- **New debug metadata.** Add it to `DebugContext` and finalize it before
  verification. Metadata must describe the IR that was actually generated.

## Verification

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

The end-to-end tests compile and run `examples/006_structs.sl`,
`examples/009_arrays.sl`, `examples/010_slices.sl`,
`examples/011_methods.sl`, `examples/012_user_methods.sl`,
`examples/013_method_borrows.sl`, `examples/014_method_namespaces.sl`,
`examples/026_tuples.sl`, and `examples/032_trait_objects.sl` through
`examples/034_trait_objects_generic.sl` in debug and release, check DWARF
sections and names, exercise object and assembly
output without `main`, verify entry-point diagnostics, confirm panic messages
and status 101 for zero divisors, signed overflow in both `/` and `%`, and
out-of-range array indices, and reject executable linking when the embedded
runtime does not match the target. `examples/007_runtime_panic.sl` pins the
user-visible panic message and location. `examples/019_match.sl` exercises unit,
positional, and named variant patterns, alternatives, a guard, and bindings in
debug and release. Tuple tests cover literals, positional access, nested access,
partial moves, borrows of one element, patterns through references, tuples in
struct fields, arrays, and enum payloads, and tuple debug information in debug
and release. Array tests also cover literals,
repeats, zero-length arrays, copy semantics, arrays in structs, structs in
arrays, function parameters, dynamic writes, and repeats inside loops. Slice
tests cover array-to-slice coercion, every range form, mutable views,
sub-slicing, struct elements, runtime length reads, bounds panics, reborrow
conflicts, and `len` shadowing. Intrinsic-method tests cover every catalog
family, chaining, statement-valued calls, empty sequences, non-`Copy` elements,
exclusive references, saturating `i128`, NaN-aware float selection, integer
clamp contracts, and `swap` bounds panics. User-method tests cover associated
constructors, shared, exclusive, and value receivers, automatic borrowing,
method/associated-function namespaces, symbol uniqueness across types, and
receiver borrow conflicts. Trait tests cover static dispatch across two impls,
trait associated functions, `Self` substitution and contract errors, ambiguity,
shared reborrow from a mutable receiver, and the rule that no method named
`main` becomes the entry point. Enum tests cover unit and payload variants,
construction into struct fields, moves through calls, associated constructors,
variant and field diagnostics, recursive and unsized variant fields, and the
generated tag/payload layout in debug and release. Trait-object tests cover
non-object-safe detection, missing and wrong-kind coercions, bare `dyn` by
value, shared-to-exclusive rejection, ambiguous supertrait methods, two concrete
implementations behind one pointer, supertrait dispatch, generic coercion, the
`{ data, vtable }` representation, vtable slot order, indirect-call lowering,
`noalias` exclusion, and the no-allocation/no-copy construction rule. Protocol
tests run `examples/037_iteration_protocols.sl` and
`examples/038_index_protocols.sl` at `O0` and `O2`, plus dynamic indexing
through `*dyn Index`. The driver tests export the
embedded archive with
`sealc emit-runtime` and reuse it with `--runtime-archive` to link that example.
