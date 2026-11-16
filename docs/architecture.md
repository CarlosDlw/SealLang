# Seal compiler architecture — foundation and lexer

This document covers the compiler foundation and the first phase, the lexer. The
foundation is the set of crates that every later phase (parser, name resolution,
type checking, MIR, codegen) depends on. It is kept in sync with the code;
changes to a public invariant must change this file in the same commit.

## Workspace layout

The compiler is a Cargo workspace. Each crate has one responsibility and is
allowed to depend only on crates below it in the list.

| Crate | Responsibility |
|-------|----------------|
| `seal-index` | Typed, non-zero `u32` indices (`newtype_index!`) and `IndexVec`. |
| `seal-hash` | Fast in-memory hashing (`FxHashMap`) and the versioned stable hash (`StableHasher`, `Fingerprint`). |
| `seal-macros` | `#[derive(StableHash)]`. Proc macros live apart so the compiler does not pay for `syn` at every rebuild. |
| `seal-symbol` | Thread-safe string interner; `Symbol` is a 4-byte handle with integer equality. |
| `seal-unicode` | Identifier classification (XID) and terminal display width. |
| `seal-span` | `SourceMap`, `SourceFile`, `LineIndex`, `Span` (8 bytes), snippets, path remapping. |
| `seal-diagnostics` | Structured diagnostics, `ErrorGuaranteed`, dedup, rich/JSON/silent emitters, ICE. |
| `seal-error-codes` | Stable code registry (`E0xxx`–`E9xxx`) with long-form explanations. |
| `seal-lexer` | Single-pass tokenizer: tokens, trivia, numeric/string/char literals, lexical diagnostics. |
| `seal-version` | `LanguageVersion` and feature gates. |
| `seal-target` | Target triples and the properties derived from them. |
| `seal-fs` | UTF-8 source loading and atomic artifact writes. |
| `seal-profiling` | Phase timings for `--time-passes`. |
| `seal-session` | Owns options, target, source map, interner, diagnostics, gates, cancellation, profiler. |
| `seal-test-support` | Inline `//~` fixture annotations and structured matching. |
| `seal-driver` | The `sealc` binary: arguments, exit codes, panic policy, and phase wiring. |

## Invariants

1. **No global mutable state.** Two sessions in one process are independent.
2. **Spans never cross files.** `SourceMap::span` validates ordering, bounds, and
   UTF-8 character boundaries. `file_of` rejects ranges covering the one-byte gap
   between files.
3. **One place counts lines.** `LineIndex` owns line breaks and line/column
   conversion; columns are one-based Unicode scalar values, with separate UTF-16
   columns for LSP.
4. **BOM is removed once.** `SourceMap::add_file` strips a leading BOM and records
   it; all offsets are relative to the stripped text.
5. **Errors are values, not strings.** Emitting an error returns
   `ErrorGuaranteed`; `Session::abort_if_errors` refuses to continue without it.
6. **Diagnostics are deterministic.** Emitters do not read the clock, environment,
   or terminal width implicitly; color is resolved by the caller and injected.
7. **Persisted hashes are stable.** Only `seal-hash`'s `StableHasher` may be used
   for cache keys or metadata. `FxHash*` is for in-memory tables only. Changing
   the stable hash requires bumping `FORMAT_VERSION`.
8. **Symbols are process-local.** The interner leaks strings until process exit and
   never serializes a `Symbol`; persist `resolve(symbol)` instead.
9. **Codes never change meaning.** New diagnostics get new codes in their phase's
   range; warnings are identified by lint name, not code.
10. **Writes are atomic.** Artifacts are written to a temporary file and renamed;
    a failed build never leaves a truncated output.
11. **Tokens are ordered and total.** The lexer always ends with an `Eof` token
    whose span is the empty range at the end of the file; token spans never
    overlap, never leave the file, and every malformed lexeme becomes one
    `Error` token covering it, so parsing can resume at a known boundary.
12. **Lexing is context-free and total.** The same text always produces the same
    tokens, independent of target or phase; the scanner always advances, and
    non-user states are reported as ICEs rather than loops. Comments are trivia
    (including nested block comments and the four doc-comment styles), and a
    newline inside any comment still sets `preceded_by_newline`.
13. **Source text is not a Trojan Source vector.** Bidirectional formatting
    characters are rejected everywhere, raw control characters are rejected in
    literals, non-ASCII whitespace and bare carriage returns are rejected, and a
    BOM is only accepted at the start of a file.

## Extension points

- **New phase.** Add a crate that depends on the session and diagnostics, exposes
  `pub fn run(session: &Session) -> Result<..., ErrorGuaranteed>`, and is called
  from `seal-driver`. Phases must not own the source map or interner; they borrow
  them from the session.
- **New diagnostic.** Allocate the next code in the phase's range in
  `seal-error-codes`, write the explanation, and emit through `DiagCtxt`. Add a
  fixture test using `seal-test-support`.
- **New token or keyword.** Add the variant to `TokenKind`, classify it in the
  scanner (keywords live in `seal-lexer`'s keyword table), and add a maximal
  munch test; keywords are reserved words, not identifiers with a flag.
- **New target.** Extend `seal-target` with a `TargetSpec` built from the triple;
  never consult the host at compile time.
- **New emitter.** Implement `seal_diagnostics::Emitter`; configuration is passed
  in, not read from the environment.
- **Incremental compilation.** The foundation already provides stable hashes,
  content hashes on `SourceFile`, and serializable IDs. A query engine can be
  added as a later crate without changing spans, symbols, or diagnostics.

## Verification

Every change must pass:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The workspace also denies `unsafe_code` and warns on missing documentation, so a
new public item without docs fails the build.
