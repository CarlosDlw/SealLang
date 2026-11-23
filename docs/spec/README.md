# Seal language specification

Status: accepted for language version `0.1` and edition `2026`. This
specification describes the surface implemented by the compiler in this
repository. It is normative: the parser, name resolver, type checker, borrow
checker, constant evaluator, and code generator must agree with it, and a
disagreement is a compiler bug or a specification bug, never a third behavior.

The specification is split by concern:

| Chapter | Covers |
|---------|--------|
| [`lexical.md`](lexical.md) | Source encoding, whitespace, comments, identifiers, keywords, tokens, and literals |
| [`grammar.md`](grammar.md) | The complete concrete grammar, precedence, associativity, and syntactic restrictions |
| [`static-semantics.md`](static-semantics.md) | Names, types, ownership, initialization, borrowing, generics, traits, `unsafe`, constant evaluation, and diagnostics |
| [`dynamic-semantics.md`](dynamic-semantics.md) | Evaluation order, values, memory, layout, ABI, panics, exit status, and undefined behavior |
| [`conformance.md`](conformance.md) | How the specification is proved against the implementation, including the test and example matrix |

## Scope

This document defines the language: what a source file means and which programs
are accepted. It covers every feature implemented in language version `0.1`:
functions, modules and imports, nominal structs and enums, tuples, arrays,
slices, references, raw pointers, generics, traits and trait objects, closures,
`Copy` and `Drop`, constants and statics, `unsafe`, C interop, `cfg`, editions,
deprecation, and the standard-library surface that the compiler treats as
built-in.

Out of scope, and specified elsewhere:

- the project/build/registry layer (`seal.toml`, lockfile, action cache,
  sandbox, remote execution) — see [`../project/`](../project/);
- compiler architecture, incremental queries, and code-generation strategy —
  see [`../architecture.md`](../architecture.md) and
  [`../codegen.md`](../codegen.md);
- the RFC and edition evolution process — see [`../rfcs/`](../rfcs/).

The compiler-internal `Error` and `Infer` types are not part of the language.
They exist only to keep recovery and inference total and must not reach an
artifact.

## Normative language

The words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are used as in RFC
2119. A rule identifier is stable: `L` is lexical, `G` is grammatical, `S` is
static semantics, `D` is dynamic semantics, and `C` is conformance. Identifiers
are never reused for a different rule. A rule may be refined by a later
edition, but the original identifier keeps its meaning for programs written
against that edition.

The numbered rules are the normative statements. Prose before a rule explains
the motivation and is informative. Code blocks are examples; they are normative
only when a rule explicitly says so. The reference tables in
[`../types.md`](../types.md), [`../memory-model.md`](../memory-model.md), and
[`../architecture.md`](../architecture.md) are normative companions for their
areas. If a companion and this specification disagree, this specification wins
and the companion MUST be corrected in the same change.

## Versions and editions

The language version is written in `project.language` and is currently exactly
`0.1`. An unknown version is rejected; it is not silently compiled as the
current one. An edition is a package-scoped compatibility line, written as
`edition = "2026"` in `seal.toml`; when absent it is the current edition,
`2026`.

An edition can change the meaning of existing syntax only through the process in
[`../rfcs/`](../rfcs/), and the change must be recorded here with the edition
that introduces it. `deprecated(...)` is metadata, not an edition switch: a
deprecated declaration remains available until an accepted RFC removes it.
Edition-dependent rules in this document are marked inline.

## Conformance

The conformance chapter maps each area to executable evidence. A conforming
implementation MUST:

1. accept every program in the positive fixtures and produce the specified
   result or diagnostic;
2. reject every program in the negative fixtures with the specified stable error
   code and primary span;
3. preserve the observable behavior of the examples at both `-O0` and `-O2`;
4. keep the grammar and semantic rules aligned with the parser and reference
   documents.

The specification is deliberately independent of compiler internals. A second
implementation can use a different parser, IR, or backend as long as it accepts
the same programs with the same observable behavior.

## Reading order

A reader implementing the language should read [`lexical.md`](lexical.md),
[`grammar.md`](grammar.md), [`static-semantics.md`](static-semantics.md), and
[`dynamic-semantics.md`](dynamic-semantics.md) in that order. A reader checking
an existing compiler should start with [`conformance.md`](conformance.md).
