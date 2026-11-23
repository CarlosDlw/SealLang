# Specification conformance

This chapter turns the specification into an executable contract. A conforming
implementation accepts the positive fixtures, rejects the negative fixtures with
the specified stable code, and preserves the behavior of every example at both
`-O0` and `-O2`.

Conformance is layered. An implementation may claim a lower layer without the
higher ones; the shipped compiler claims all layers.

| Level | Claim |
|-------|-------|
| C1 — Lexical | Source text produces the tokens, literals, and lexical diagnostics in [`lexical.md`](lexical.md) |
| C2 — Grammar | Accepted files derive from [`grammar.md`](grammar.md), and invalid files are rejected or recovered only for tooling |
| C3 — Static | Names, types, ownership, initialization, borrowing, patterns, `unsafe`, constants, and FFI follow [`static-semantics.md`](static-semantics.md) |
| C4 — Dynamic | Evaluation, memory, layout, ABI, destruction, panics, and undefined behavior follow [`dynamic-semantics.md`](dynamic-semantics.md) |
| C5 — Editions | `deprecated(...)`, lint control, and package editions follow S56–S57 and the edition policy |

## Grammar audit

The concrete grammar was audited against the parser modules that implement it.
The mapping is deliberate: every production has one owner, and a change to a
production must change its owning parser module and its tests in the same
commit.

| Grammar area | Implementation | Primary tests |
|--------------|----------------|---------------|
| `file`, `module-decl`, `use-decl` | `crates/parser/src/parser.rs`, `module.rs` | `crates/parser/src/tests/{modules,imports}.rs` |
| `item-modifiers`, `deprecation`, `cfg` | `crates/parser/src/modifiers.rs`, `cfg.rs` | `crates/parser/src/tests/{visibility,deprecated,cfg}.rs` |
| Functions, constants, statics | `crates/parser/src/parser.rs` | `crates/parser/src/tests/{items,stmts,globals}.rs` |
| Structs, enums, variants, fields | `crates/parser/src/parser.rs`, `extern_item.rs` | `crates/parser/src/tests/{structs,enums,tuple_variants,extern_items}.rs` |
| Type aliases | `crates/parser/src/parser.rs` | `crates/parser/src/tests/type_aliases.rs` |
| Traits, impls, methods | `crates/parser/src/parser.rs`, `generics.rs` | `crates/parser/src/tests/{traits,impls,methods,generics}.rs` |
| External declarations | `crates/parser/src/extern_item.rs` | `crates/parser/src/tests/{extern_items,fn_sigs}.rs` |
| Generic parameters, arguments, bounds, `where` | `crates/parser/src/generics.rs` | `crates/parser/src/tests/generics.rs` |
| Types, references, raw pointers, `dyn`, function types | `crates/parser/src/types.rs` | `crates/parser/src/tests/{fn_ptrs,dyn_types,arrays,slices,tuples}.rs` |
| Expressions, precedence, calls, casts | `crates/parser/src/expr.rs` | `crates/parser/src/tests/{exprs,casts,closures,methods}.rs` |
| Blocks, statements, loops | `crates/parser/src/stmt.rs` | `crates/parser/src/tests/{stmts,for_loop,if_let,while_let}.rs` |
| Patterns and match arms | `crates/parser/src/pattern.rs` | `crates/parser/src/tests/{match_expr,match_advanced}.rs` |
| Lexical tokens, literals, trivia | `crates/lexer/src/*` | `crates/lexer/src/tests/{tokens,numbers,literals,trivia,recovery}.rs` |

The grammar is an acceptance grammar, not a description of recovery. `E1001`
through `E1014` and the external/modifier codes are the normative rejection
codes; the parser's total tree and synchronization points are implementation
requirements for editors (G45). A recovered file is never a conforming program.

## Static evidence

| Area | Rules | Evidence |
|------|-------|----------|
| Modules, imports, re-exports, visibility | S1–S8, S13 | `crates/hir/src/tests/{modules,imports,reexports,prelude,api,packages}.rs`; `examples/multi-module`, `examples/workspace` |
| Namespaces, scopes, definitions | S9–S12 | `crates/hir/src/tests/{resolve,modules,imports}.rs`; `crates/parser/src/tests/items.rs` |
| Types, generics, traits, object safety | S14–S27 | `crates/typeck/src/tests/{types,generics,traits,dyn_types,fn_traits,fn_ptrs,sized}.rs`; `crates/hir/src/tests/{traits,generics,dyn_types}.rs` |
| Expressions, coercions, casts, calls | S28–S38 | `crates/typeck/src/tests/{operators,casts,calls,methods,fn_ptrs,fn_traits,closures,indexing_protocol}.rs` |
| Ownership, initialization, drops | S39–S41, S44 | `crates/borrowck/src/tests/{drops,drop_rules,copy_optin,constructors}.rs`; `crates/typeck/src/tests/{init,drop_builtin,forget_builtin}.rs`; `crates/hir/src/tests/{copy_impls,drop_impls,drop_builtin}.rs` |
| Borrowing and global effects | S42–S43 | `crates/borrowck/src/tests/{borrows,calls,arrays,closures}.rs`; `crates/driver/tests/global_diagnostics.rs` |
| Patterns, match, loops | S45–S48 | `crates/typeck/src/tests/{match_expr,match_advanced,slice_patterns,reference_patterns,for_loop,if_let,while_let}.rs` |
| Unsafe, raw pointers, C interop | S49–S52 | `crates/typeck/src/tests/{raw_pointers,raw_deref,raw_casts,raw_methods,ffi,c_types}.rs`; `crates/driver/tests/linking.rs`; `examples/` FFI examples |
| Constants, statics, evaluation | S53–S55 | `crates/const-eval` tests; `crates/driver/tests/global_diagnostics.rs`; `examples/039_const_static.sl` |
| Deprecation, lints, editions | S56–S57 | `crates/parser/src/tests/deprecated.rs`; `crates/hir/src/tests/deprecated.rs`; `crates/typeck/src/tests/deprecated.rs`; `crates/driver/tests/diagnostics.rs`; `examples/deprecation` |
| Type aliases | S61–S65 | `crates/parser/src/tests/type_aliases.rs`; `crates/hir/src/tests/type_aliases.rs`; `crates/driver/tests/type_aliases.rs` |

## Dynamic evidence

| Area | Rules | Evidence |
|------|-------|----------|
| Evaluation order, short-circuiting, calls | D1–D9 | `crates/typeck/src/tests/{operators,calls,methods,closures}.rs`; `crates/driver/tests/examples.rs`; `examples/` numbered programs |
| Integer and float semantics | D9–D12 | `crates/typeck/src/tests/{operators,casts}.rs`; `crates/codegen/tests/end_to_end.rs`; `examples/035_numeric_casts.sl` |
| Equality and layout | D11, D19 | `crates/ty/src` and `crates/layout/src` tests; `crates/codegen/tests/end_to_end.rs`; `docs/types.md` layout fixtures |
| References, raw pointers, UB contracts | D13–D16 | `crates/borrowck/src/tests/borrows.rs`; `crates/typeck/src/tests/raw_*.rs`; `crates/const-eval` raw tests |
| Drop order and overwrite | D17–D18 | `crates/borrowck/src/tests/{drops,drop_rules}.rs`; `crates/codegen/tests/end_to_end.rs` |
| ABI, C interop, entry point, exit | D19–D22 | `crates/codegen/tests/end_to_end.rs`; `crates/driver/tests/{linking,runtime_archive}.rs`; `examples/freestanding` |
| Constants and statics at run time | D23–D24 | `crates/const-eval` tests; `crates/driver/tests/global_diagnostics.rs` |
| Determinism and optimization agreement | D25, D28–D30 | `crates/driver/tests/examples.rs`; `crates/cli/tests/project_cli.rs::verify_repro_builds_two_copies_and_finds_them_identical`; `examples/README.md` |

## Commands

Run the focused suites while changing a chapter:

```console
$ cargo test -p seal-lexer -p seal-parser
$ cargo test -p seal-hir -p seal-typeck
$ cargo test -p seal-borrowck -p seal-const-eval
$ cargo test -p seal-driver -p seal-codegen
```

Run the complete gate before declaring conformance:

```console
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets --locked -- -D warnings
$ cargo build --workspace --locked
$ RUST_MIN_STACK=8388608 cargo test --workspace --locked
$ RUST_MIN_STACK=8388608 cargo test --workspace --locked -j 1 -- --test-threads=1
$ target/debug/seal stdlib verify
$ target/debug/seal --project examples/multi-module verify-repro
$ RUST_MIN_STACK=8388608 RUST_TEST_THREADS=1 CARGO_BUILD_JOBS=1 scripts/verify-commit.sh HEAD
```

The larger `RUST_MIN_STACK` is required by the deep-nesting parser test on a
2 MB test-thread stack; it is an environment setting, not a language rule. The
serial suite and `verify-commit.sh` are the final gates because they also cover
order-dependent state and commit isolation.

## Negative and boundary coverage

The conformance claim is not only the happy path. The following must stay green:

- lexical malformed input: unterminated strings/comments, invalid escapes,
  numeric separators, suffixes, BOM, bare CR, bidi characters, and control
  characters (`crates/lexer/src/tests/{recovery,literals,numbers,trivia}.rs`);
- grammar boundaries: empty generic lists, missing delimiters, mixed variant
  fields, module/import position, invalid modifiers, malformed `cfg`, and
  recovery (`crates/parser/src/tests/{errors,recovery,visibility,cfg,imports}.rs`);
- static failures: wrong namespaces, private access, missing trait methods,
  overlapping impls, non-exhaustive matches, moved values, uninitialized reads,
  aliasing conflicts, unsupported casts, non-`Sized` storage, invalid FFI, and
  alias cycles/arity/namespace/bounds failures
  (`crates/typeck/src/tests`, `crates/borrowck/src/tests`,
  `crates/driver/tests/*_diagnostics.rs`, `crates/hir/src/tests/type_aliases.rs`);
- dynamic boundaries: panic status 101, freestanding trap, bounds checks,
  division by zero, drop order, `mem::forget`, and `-O0`/`-O2` agreement
  (`crates/driver/tests/examples.rs`, `examples/`).

## Changing the specification

A change to the language contract MUST update, in one change:

1. the owning rule in this directory;
2. the parser, checker, or runtime implementation it describes;
3. the positive and negative tests that prove it;
4. the English guide and reference table when behavior is user-visible;
5. the RFC or edition entry when the change is not backward-compatible.

A rule identifier is never reused. A correction that changes meaning gets a new
rule or an explicit edition note. A test is evidence only when it fails for the
old behavior and passes for the new one; a compile-only check does not prove a
dynamic rule.

## Explicit non-goals

This specification does not define a mechanized proof (Coq, Lean, K), a
regulatory certification, a stable external ABI for Seal function pointers and
vtables, or the project/build/registry layer. Those are separate contracts.
Mechanized verification would be an additional assurance layer, not a
replacement for the executable conformance suite above.
