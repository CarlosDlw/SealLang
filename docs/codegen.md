# Seal code generation

`seal-codegen` is the only crate that turns checked MIR into machine code. It
produces one artifact per invocation:

- a relocatable object (`--emit=obj`),
- target assembly (`--emit=asm`), or
- a linked native executable (`--emit=exe`, the default).

`--emit=check` stops after borrow checking and writes nothing. Object and
assembly output do not require an entry point; executables do.

## Pipeline

1. **Declare.** Every MIR body becomes a `seal.<name>` LLVM function with
   external linkage and the signature derived from its argument and return
   types. Declaring first lets a body call a function declared later.
2. **Lower.** Each body gets one stack slot per local, one LLVM block per MIR
   block, and explicit loads/stores for places. Field projections use struct
   GEPs; dereferences load the reference and continue from its pointee.
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
sealc [-O0..3] [-g] [--emit=check|obj|asm|exe] [-o PATH] file.sl
```

- Without `-o`, `file.sl` produces `file` for an executable, `file.o` for an
  object, or `file.s` for assembly, next to the input.
- `-o` accepts one output path and is rejected with more than one input.
- `-g` embeds DWARF in objects and executables; assembly contains the matching
  `.file`/`.loc` directives.
- `--emit=check` is the fast path for editors and CI.

The linker is selected from `SEAL_LINKER`, then `CC`, then `cc`. Arguments are
passed directly, never through a shell. A linker failure is `E6003` and includes
the linker's stderr.

## ABI and layout

The ABI is defined by the language, not by a C header:

- Seal functions have external visibility and the symbol `seal.<name>`; the C
  `main` wrapper is the only symbol named `main`.
- A user `main` must take no parameters and return `i32` or `()`. The wrapper
  always returns `i32`, using the returned value or zero for unit. A missing or
  invalid entry point is `E6001`.
- Locals, parameters, and the return place live in `alloca` slots. Reading and
  writing a place is a load or store; moving is a bitwise transfer because no
  current type has a destructor.
- Structs are LLVM structs with fields in declaration order, natural alignment,
  and no reordering. Scalars use their size as alignment, references use the
  target pointer width, `bool` is one byte, and `()`/`!` are zero-sized structs.
  The layout comes from `seal-layout`, so code generation and semantic layout
  cannot disagree.
- `*str` is the fat pointer `{ data, length }`. A literal becomes a private,
  unnamed constant array plus its byte length; no run-time terminator scan is
  needed.
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
- pointer and fat-pointer types, including `&str` as a two-member struct;
- a line-table entry for every statement and terminator, so debuggers stop on
  source lines rather than function boundaries.

Recursive structs use forward declarations for the pointee of a reference; a
struct cannot contain itself by value, so this models every legal recursion.

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

The end-to-end tests compile and run `examples/006_structs.sl` in debug and
release, check DWARF sections and names, exercise object and assembly output
without `main`, verify entry-point diagnostics, and confirm panic messages and
status 101 for zero divisors and signed overflow in both `/` and `%`, and reject
executable linking when the embedded runtime does not match the target.
`examples/007_runtime_panic.sl` pins the user-visible panic message and location.
The driver tests export the embedded archive with `--emit-runtime` and reuse it
with `--runtime-archive` to link that example.
