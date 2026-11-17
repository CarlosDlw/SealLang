# Seal code generation

`seal-codegen` is the only crate that turns checked MIR into machine code. It
produces one artifact per invocation:

- a relocatable object (`--emit=obj`),
- target assembly (`--emit=asm`), or
- a linked native executable (`--emit=exe`, the default).

`--emit=check` stops after borrow checking and writes nothing. Object and
assembly output do not require an entry point; executables do.

## Pipeline

1. **Declare.** Every MIR body becomes an LLVM function with external linkage
   and the signature derived from its argument and return types. A free
   function is `seal.<name>`; an impl function is `seal.<Type>.<name>`, so
   same-named methods on different types stay distinct. Declaring first lets a
   body call a function declared later.
2. **Lower.** Each body gets one stack slot per local, one LLVM block per MIR
   block, and explicit loads/stores for places. Field projections use struct
   GEPs; array indices use a byte offset after the MIR bounds check; dereferences
   load the reference and continue from its pointee.
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
sealc [OPTIONS] <INPUT>...
sealc --emit-runtime <PATH>
sealc --explain <CODE>
```

`--help` lists every option grouped by concern: output, optimization, target,
diagnostics, and runtime. Options accept `--name value` and `--name=value`, and
`--` ends option parsing so a file named like a flag stays an input.

- `--emit=exe` (the default) links an executable, `--emit=obj` writes an object,
  `--emit=asm` writes assembly, and `--emit=check` stops after borrow checking
  and writes nothing.
- Without `-o`, `file.sl` produces `file` for an executable, `file.o` for an
  object, or `file.s` for assembly, next to the input.
- `-o` accepts one output path and is rejected with more than one input.
- `-O` alone selects level 3; `-O0` through `-O3` and `--opt-level <LEVEL>`
  select a level explicitly.
- `-g` embeds DWARF in objects and executables; assembly contains the matching
  `.file`/`.loc` directives.
- `--emit-runtime <PATH>` writes the embedded runtime archive without compiling,
  and `--runtime-archive <PATH>` links against a different archive.
- `--explain <CODE>` prints an error-code explanation without compiling.

The exit status is 0 on success, 1 after a compile error, 2 after a usage error,
and 101 after an internal compiler error or a runtime panic.

The linker is selected from `SEAL_LINKER`, then `CC`, then `cc`. Arguments are
passed directly, never through a shell. A linker failure is `E6003` and includes
the linker's stderr.

## ABI and layout

The ABI is defined by the language, not by a C header:

- A free Seal function has external visibility and the symbol `seal.<name>`;
  an impl function, inherent or from a trait implementation, uses
  `seal.<Type>.<name>`. The C `main` wrapper is the only symbol named `main`,
  and only a free function can be the language entry point.
- A user `main` must take no parameters and return `i32` or `()`. The wrapper
  always returns `i32`, using the returned value or zero for unit. A missing or
  invalid entry point is `E6001`.
- Locals, parameters, and the return place live in `alloca` slots. Reading and
  writing a place is a load or store; moving is a bitwise transfer because no
  current type has a destructor.
- Structs are LLVM structs with fields in declaration order, natural alignment,
  and no reordering. Arrays are LLVM arrays of their element type: the element
  size already includes its trailing padding, so the array stride matches
  `seal-layout`. Scalars use their size as alignment, references use the target
  pointer width, `bool` is one byte, and `()`/`!` are zero-sized structs. The
  layout comes from `seal-layout`, so code generation and semantic layout cannot
  disagree.
- An enum is an LLVM struct `{ tag, anchor, payload }`: the tag is the smallest
  integer that addresses every variant, a zero-length array of payload-aligned
  integers contributes the payload alignment without occupying storage, and the
  payload is a byte array as large as the largest variant. Constructing a
  variant stores the discriminant and then stores a variant-shaped struct at the
  payload address; the temporary lives in the function prologue, so a loop does
  not grow the stack. Every variant shares the same LLVM type, and the payload
  bytes at the language offsets are what `match` will later read.
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
- Slice indexing reloads the data member of the fat pointer and checks the
  runtime index against the length member before the element offset. `len`
  extracts the length member directly; both operations are O(1).
- User methods lower from `Rvalue::Call`; the receiver is the first argument.
  An automatic shared or exclusive receiver borrow is an explicit
  `Rvalue::Ref` in MIR, so code generation emits the ordinary address and load
  for the call. Associated functions are ordinary calls reached through a
  resolved function path. Trait methods take the same path because dispatch is
  static, and a `*mut T` receiver passed to a shared method becomes a shared
  `Rvalue::Ref` of the pointee.
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
- A reference is one pointer-sized word in both kinds: `*T` and `*mut T` share a
  representation and differ only in the checks they enable, so the kind costs
  nothing at run time. The backend marks an exclusive-reference parameter
  `noalias` only when borrow checking proved the whole program upholds its
  exclusive grant: the function body never reaches storage through a reference
  of unknown target, and every call site proves its reference arguments name
  disjoint storage. The proof covers every call site in the compiled program;
  calling an exported function from foreign code with aliasing pointers is
  outside it.  A shared `*T` parameter is `readonly`: the type system forbids
  writing through it, so that promise needs no whole-program proof. A `*str`,
  `*[T]`, or `*mut [T]` parameter is a fat pointer, which LLVM cannot annotate,
  so it keeps the ordinary ABI.
- The object is position independent (`RelocMode::PIC`) and uses the target's
  default code model.

## Debug information

`-g` requests `DWARFEmissionKind::Full` with `DWARFSourceLanguage::Rust`:

- one compile unit per input file, with the displayed (possibly remapped) path;
- one `DW_TAG_subprogram` per Seal function, linked to the LLVM function;
- `DW_TAG_formal_parameter` for parameters and `DW_TAG_variable` for bindings,
  declared at their stack slots with source line and column;
- `DW_TAG_structure_type` for every struct, with member names, bit offsets, and
  sizes from `seal-layout`;
- `DW_TAG_structure_type` for every enum with its discriminant member and a
  payload byte-array member at the layout offset. Variant parts are not emitted
  yet, so a debugger shows the tag and raw payload rather than the active
  variant's fields; the layout is still exact;
- pointer and fat-pointer types, including `&str` and `&[T]` as two-member
  structs;
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
- `sealc --emit-runtime <PATH>` writes the embedded archive for manual links of
  object files produced with `--emit=obj`.
- The archive is built for the compiler host. `--emit=exe` for another
  architecture or OS is rejected with `E6003` unless `--runtime-archive <PATH>`
  supplies an archive built for that target. `--emit=obj` remains available for
  cross-compilation.

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
  sysroot, and runtime archive, while `--emit=obj` works without them.
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
`examples/013_method_borrows.sl`, and `examples/014_method_namespaces.sl` in
debug and release, check DWARF sections and names, exercise object and assembly
output without `main`, verify entry-point diagnostics, confirm panic messages
and status 101 for zero divisors, signed overflow in both `/` and `%`, and
out-of-range array indices, and reject executable linking when the embedded
runtime does not match the target. `examples/007_runtime_panic.sl` pins the
user-visible panic message and location. Array tests also cover literals,
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
generated tag/payload layout in debug and release. The driver tests export the
embedded archive with
`--emit-runtime` and reuse it with `--runtime-archive` to link that example.
