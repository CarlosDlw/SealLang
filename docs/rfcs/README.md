# Seal RFCs

An RFC is the written record of a change to the language contract. It exists so
that a semantic decision has a motivation, a specification, rejected
alternatives, a migration story, and evidence before code makes it permanent.

The process is deliberately small. It is not a second issue tracker and it does
not replace design review in a pull request. It is the place where a decision
that affects users is written down once and can be cited later.

## When an RFC is required

Open an RFC before implementation when a change does any of the following:

- adds, removes, or changes syntax, name resolution, type checking, ownership,
  initialization, destruction, layout, ABI, or a safe-code guarantee;
- changes the standard-library contract, the prelude, or a compiler builtin;
- changes the manifest, lockfile, compile-plan, registry, or public-API
  contract in a way users or tools can observe;
- creates a new edition or removes something deprecated at an edition
  boundary;
- introduces a new diagnostic category, lint, or deprecation policy;
- is large enough to need more than one reviewable commit and a migration
  guide.

An RFC is not required for a bug fix that restores documented behavior, a
refactor with no observable change, an internal performance improvement, a
test-only change, or a documentation correction. When in doubt, ask in the
change first; a short RFC is cheaper than an unrecorded convention.

## Lifecycle

| Status | Meaning | Who moves it |
|--------|---------|--------------|
| `Draft` | The author is still shaping the proposal. It may be incomplete. | Author |
| `Proposed` | The proposal is complete enough for review. | Author |
| `Accepted` | The design is approved. Implementation may start. | Maintainer after review |
| `Rejected` | The proposal was declined. The record and reasons stay. | Maintainer after review |
| `Withdrawn` | The author removed the proposal before a decision. | Author |
| `Implemented` | The accepted design is in the repository and documented. | Implementer, with evidence |

A status change is a one-line edit plus the decision record. An accepted RFC is
not rewritten to match a different implementation: if the design changes, open
a new RFC that supersedes it and link both directions.

## File layout

- `NNNN-kebab-title.md`, where `NNNN` is the next unused four-digit number.
- `0001-editions-and-deprecation.md` is the first accepted example.
- `template.md` is the canonical form. Copy it; do not invent a different
  section order.

Numbers are never reused, including for rejected or withdrawn proposals. The
file stays in the repository so the decision history is complete.

## Required sections

Every proposed RFC contains, in this order:

1. **Summary** - what changes, in one paragraph.
2. **Motivation** - the user-visible problem and why the current surface is
   insufficient.
3. **Guide-level explanation** - how a user writes and reasons about the
   feature, with examples.
4. **Reference-level explanation** - the exact rules, states, errors, and
   interactions with ownership, aliasing, initialization, destruction, layout,
   ABI, constant evaluation, and tooling.
5. **Drawbacks** - the real costs, not a formality.
6. **Rationale and alternatives** - why this design, and what was rejected.
7. **Prior art** - how other languages or systems solve it.
8. **Unresolved questions** - what must be decided before acceptance. An RFC
   with a blocking unresolved question is not `Proposed`.
9. **Edition and deprecation impact** - whether the change needs a new edition,
   a deprecation period, or both, and what migration users need.
10. **Compatibility and migration** - source, manifest, plan, lockfile, ABI,
    and tooling compatibility, including rollback.
11. **Security and safety** - trust boundaries, new input, `unsafe`, and the
    effect on existing guarantees.
12. **Testing and acceptance** - the matrix of happy paths, errors, edges, and
    negative cases that will prove the change.
13. **Implementation plan** - phases with implementation, tests, and exit
    criteria, each independently reviewable and committable.
14. **Future possibilities** - what this deliberately leaves open, and how the
    next change extends it without a redesign.

## Review checklist

A reviewer checks all of these before `Accepted`:

- [ ] The motivation names a real user problem, not only an implementation
      preference.
- [ ] The guide-level explanation is enough to write correct code without
      reading the compiler.
- [ ] The reference-level rules cover ownership, aliasing, initialization,
      destruction, layout, ABI, and constant evaluation where relevant.
- [ ] Every new error or warning has a stable name or code and a message that
      says what to do.
- [ ] The design does not weaken a safe-code guarantee. If it intentionally
      changes one, the change is an edition and the RFC says so.
- [ ] The edition impact is explicit: no edition needed, a new edition, or a
      deprecation with a removal edition.
- [ ] Migration and rollback are concrete, not "update your code".
- [ ] The test matrix includes negative cases and edge conditions, not only the
      happy path.
- [ ] The implementation phases have exit criteria and no phase leaves a
      public surface that a later phase makes correct.
- [ ] Documentation, examples, and release notes are part of the plan.
- [ ] The proposal leaves room for the next feature instead of closing an
      interface prematurely.
- [ ] The design can be explained in one page to a user who has not read the
      compiler.

## Decisions

The maintainer decides. The decision is recorded in the RFC's status line and
in a short "Decision" section that names the date, the accepted shape, and any
conditions. Review aims for consensus; a lack of objections is not consensus.
When the decision is contested, the RFC stays `Proposed` until the trade-off is
resolved or the proposal is rejected.

The implementation plan in an accepted RFC is a contract, not a suggestion.
Scope changes discovered during implementation are handled by updating the
plan and asking for a decision when they change semantics, a public interface,
compatibility, or security. They are never hidden in a commit.

## Editions and deprecation

Editions and deprecation are the two mechanisms that make evolution safe:

- **Editions** are year-named compatibility lines selected per package. A new
  edition is created only by an accepted RFC, and that RFC lists every breaking
  change and its migration.
- **Deprecation** keeps an item available while warning users. Removal happens
  only at an edition boundary named by an accepted RFC, with a release note and
  a migration recipe.

The full policy is in
[`0001-editions-and-deprecation.md`](0001-editions-and-deprecation.md).

## Implementing an accepted RFC

1. Read the RFC and its decisions again before writing code.
2. Add the RFC to the release notes when the change becomes user-visible.
3. Follow the phases. Each phase has its own tests, gate, and commit.
4. Update the RFC status to `Implemented` only after the definition of done is
   verified.
5. If the implementation diverges, stop and amend the RFC through a new
   proposal, not a silent edit.

## Index

| RFC | Title | Status |
|-----|-------|--------|
| [0001](0001-editions-and-deprecation.md) | Editions and deprecation | Accepted |
