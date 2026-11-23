# RFC NNNN: Title

- Status: Draft
- Implementation: not started
- Date: YYYY-MM-DD
- Supersedes: none
- Superseded by: none

## Summary

One paragraph. State what changes and who it affects. Do not describe the
implementation yet.

## Motivation

The user-visible problem. What can a user not express, not prove, or not
migrate today? Include concrete examples and the cost of leaving it unchanged.
An implementation preference is not a motivation.

## Guide-level explanation

Explain the feature as a user would learn it. Show the smallest complete
example, the normal form, and the error a user sees when they get it wrong.
Name the concepts the rest of the document uses.

## Reference-level explanation

The exact contract:

- grammar and placement;
- name resolution and typing;
- ownership, aliasing, initialization, and destruction;
- layout and ABI;
- constant evaluation;
- diagnostics, including stable names or codes;
- interaction with packages, editions, lints, docs, and tooling.

State invalid states explicitly. If the feature changes an invariant, name the
invariant and the edition that owns the change.

## Drawbacks

The real costs: extra syntax, a larger compiler surface, a migration burden, a
performance risk, or a new concept users must learn. Do not list costs that do
not apply.

## Rationale and alternatives

Why this design. List the serious alternatives and why each was rejected.
Include the "do nothing" option and its cost.

## Prior art

How other languages, compilers, or systems solve the same problem. Note what
transfers and what does not.

## Unresolved questions

Every blocking question, with the decision needed and who must decide it. An
RFC with a blocking question is not ready for `Proposed`.

## Edition and deprecation impact

- Does this need a new edition? If yes, name it and list the breaking change.
- Does an existing surface become deprecated? Give the `since` release, the
  note or replacement, and the edition that may remove it.
- Does this change a package's edition-independent behavior? Explain why not.

"No edition and no deprecation" is a valid answer, but it must be stated.

## Compatibility and migration

Cover source, manifest, compile plan, lockfile, public API, ABI, registry, and
tooling. Give the migration steps and the rollback plan. A breaking change
without a migration is not ready.

## Security and safety

Name every new trust boundary and every new input. State whether the feature
introduces `unsafe`, filesystem, network, process, or secret handling. Explain
why the safe-code guarantees are unchanged or which guarantee changes and in
which edition.

## Testing and acceptance

The matrix: happy path, error paths, boundaries, empty and maximum inputs,
invalid states, negative cases, and compatibility cases. Name the layer that
proves each row (unit, integration, end-to-end, artifact inspection). A test
that only compiles is not evidence of behavior.

## Implementation plan

Phases in dependency order. Each phase has:

- **Implementation** - files, interfaces, and behavior.
- **Tests** - what proves it.
- **Exit criteria** - the observable state that lets the next phase start.

Each phase must be independently reviewable, testable, and committable. No
phase leaves a `TODO`, a stub, a crash, or a public surface that a later phase
makes correct.

## Future possibilities

What this deliberately leaves open, and how a later feature extends it without
a destructive refactor. State what is out of scope and why.
