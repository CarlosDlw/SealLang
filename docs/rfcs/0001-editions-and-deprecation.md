# RFC 0001: Editions and deprecation

- Status: Accepted
- Implementation: in progress
- Date: 2026-11-23
- Supersedes: none
- Superseded by: none

## Summary

Seal gains two mechanisms for changing the language without breaking existing
code by accident. **Editions** are year-named compatibility lines selected per
package; the first is `2026`. **Deprecation** lets a declaration warn every use
with a release version and a replacement, while removal is scheduled only at an
edition boundary.

The change also establishes the RFC process itself. A language or contract
change is proposed in `docs/rfcs/`, reviewed, and recorded before it becomes
permanent.

## Motivation

Seal is pre-1.0, but its programs already have dependencies, lockfiles, API
snapshots, and caches. A breaking language change cannot be a silent release
note:

- a dependency must be able to keep the semantics it was written against;
- a user must be able to see that an item is going away before it disappears;
- a tool must be able to tell whether a warning is allowed, denied, or only
  informational;
- a reviewer must be able to find the decision that made a construct mean what
  it means.

Today `[project] language = "0.2"` is accepted by the manifest and then ignored,
so a project can claim a version the compiler does not implement. There is no
edition, no deprecation syntax, no deprecation warning, and no lint name for an
individual warning. This RFC fixes those gaps with one coherent contract.

## Guide-level explanation

### Selecting an edition

A package declares its edition in `seal.toml`:

```toml
[package]
name = "app"
version = "0.1.0"
edition = "2026"
```

The field is optional. When it is absent, the package uses the current edition,
so existing manifests keep working. `seal new` and `seal init` write it
explicitly. `seal metadata` reports it for every package.

An edition is not the compiler version and not the language version.
`project.language` names the language surface the project expects (`0.1`); an
edition names the compatibility line for breaking changes. The two are
validated separately.

### Marking a declaration deprecated

```seal
deprecated(since: "0.2.0", note: "use new_api")
public func old_api() -> i32 {
  0
}

struct Config {
  deprecated(since: "0.2.0", replacement: "new_field")
  old_field: i32,
  new_field: i32,
}
```

`deprecated` is a contextual modifier, like `test`, `bench`, and `public`. It
is only consumed at the start of a declaration, so it remains a valid name
everywhere else.

`since` is required and must be a semantic version. `note` and `replacement`
are optional guidance. A declaration may have one deprecation modifier.

Deprecation is allowed on functions, constants, statics, structs, enums,
opaque types, traits, trait methods, inherent methods, fields, variants, and
external declarations. It is rejected on modules, imports, and impl blocks.

### What a user sees

Using a deprecated item produces one warning at the use site:

```text
warning: `old_api` is deprecated since 0.2.0
  --> src/main.sl:8:3
   |
 8 |   old_api();
   |   ^^^^^^^ this use is deprecated
   |
  --> src/lib.sl:2:8
   |
 2 | public func old_api() -> i32 {
   |        ^^^^^^^ the function was deprecated here
   |
   = help: use new_api
```

The warning has the stable lint name `deprecated`. `seal lint --allow
deprecated` suppresses it; `seal lint --deny deprecated` turns it into an error
and fails the command. `--deny warnings` still denies every warning.

A use inside the deprecated declaration itself does not warn. A use in a
`cfg`-inactive item does not warn because it is never lowered.

### Removing an item

Deprecation is not removal. An item stays available for at least one minor
release line and warns for its whole deprecated life. Removal happens only in
an edition named by an accepted RFC, with a release note and a migration
recipe. The compiler does not turn `deprecated` into a hard error by itself.

## Reference-level explanation

### Edition type and selection

`seal_version::Edition` is a year value:

```rust
pub struct Edition(u16);

impl Edition {
    pub const CURRENT: Self = Self(2026);
    pub const ALL: &'static [Self] = &[Self::CURRENT];
    pub fn parse(text: &str) -> Result<Self, EditionError>;
}
```

- `Edition::parse` accepts exactly four decimal digits and rejects anything
  else, including `2026.1`, `26`, an empty string, and a non-numeric value.
- A value not in `Edition::ALL` is an unsupported edition.
- `Display` writes the four-digit year.
- Ordering is numeric, so `2029 > 2026`.

`[package] edition` is a string in the manifest and is validated at load time.
An unknown value is `E8005` with the supported list. An absent value is
`Edition::CURRENT`.

`project.language` is validated against `LanguageVersion::CURRENT` exactly.
`0.1` is accepted; `0.2` is rejected with `E8005` until a compiler implements
it. The old behavior ignored the value, which silently compiled a `0.2`
manifest as `0.1`.

### Editions in the compile plan

`CompilePlan` schema version is `3`. `PlanPackage` gains:

```json
"edition": "2026"
```

The driver validates every package's edition with `Edition::parse` and rejects
an unsupported value as `E8001`. A version `2` plan is still readable: every
package is upgraded to `Edition::CURRENT` in memory. A version greater than `3`
is `E8001`. `seal` writes version `3`.

`PackagePlanEntry` carries the edition into `ModuleGraph`, which stores it in
`PackageData`. A multi-package compilation therefore keeps one edition per
package; a dependency is never re-editioned by its dependent.

The parser does not take an edition parameter yet. The current edition changes
no syntax. When an accepted RFC introduces syntax that depends on an edition,
the parser gains an `Edition` parameter, and the driver passes each file's
package edition before building the AST. Semantic gates already have the
edition in `PackageData`.

### Edition gates

`Edition` exposes a closed feature table:

```rust
pub enum EditionFeature {
    // One variant per breaking change ratified by an RFC.
}

impl Edition {
    pub const fn supports(self, feature: EditionFeature) -> bool;
}
```

The table is empty in edition `2026`. That is deliberate: there is no
breaking change to gate yet, and inventing one would change the language
without an RFC. The first accepted breaking-change RFC adds its variant, the
new edition, the gate, the migration, and the tests.

### Deprecation syntax

```ebnf
deprecation = "deprecated" "(" entry { "," entry } ")" ;
entry       = "since" ":" string
            | "note" ":" string
            | "replacement" ":" string ;
```

- Entries may appear in any order.
- `since` is required exactly once.
- `note` and `replacement` are optional, each at most once.
- Unknown keys, duplicate keys, a missing `since`, an invalid semantic
  version, an empty value, a value with a control character, or a value over
  256 bytes are `E1001`.
- A deprecation modifier on a module, import, or impl block is `E2079`.
- Positional tuple fields have no name to point at and are not deprecatable.
- `deprecated` is contextual: outside a declaration prefix it is an ordinary
  identifier.

`seal_def::Deprecation` stores interned `since`, `note`, and `replacement`
symbols plus the modifier span. `DefData.deprecated` stores the value for every
deprecatable definition. `FieldDecl`, `VariantDecl`, and `TraitMethodDecl`
carry it directly because they do not store a full `Modifiers`.

### Use-site warning

`Lowering::warn_deprecated(def, use_span)` emits a warning with lint name
`deprecated` and:

- a primary label at the use;
- a secondary label at the definition's name;
- the `since` version in the message;
- a help line with the note or replacement when present.

The warning is suppressed when `use_span` is inside the deprecated
definition's span. It is emitted once per use; `DiagCtxt` deduplicates by
message, level, and primary span. A `cfg`-inactive item is not lowered and
therefore produces no warning.

The hook is called from every resolution site that commits to a `DefId` for a
source-level use: unqualified value, type, and trait resolution; qualified
paths; method calls; field access; struct literals; patterns; variant paths;
imports and re-exports; trait impl headers; external items; and type
arguments.

### Lint names and levels

`Diagnostic` gains `lint: Option<&'static str>`. `DiagCtxt` gains a table from
lint name to `LintLevel`:

- `Allow` drops the diagnostic;
- `Warn` emits it as a warning (the default);
- `Deny` emits it as an error, so the build fails.

The driver accepts repeatable `--allow-lint NAME` and `--deny-lint NAME`.
`seal lint --allow` and `--deny` map to those flags; the `warnings` group is a
command-level alias for denying every lint, preserving today's
`--deny warnings`. Unknown names are `E8112`; the same name in both lists is
`E8112`.

The first registered lints are `unused_import` and `deprecated`. This is a
bounded implementation of lint levels for the warnings that exist, not the full
attribute and lint-level system of a later RFC.

### Public API and documentation

`seal api` adds a `deprecated` object to a public item when present:

```json
"deprecated": {
  "since": "0.2.0",
  "note": "use new_api",
  "replacement": "new_api"
}
```

`seal doc` renders a "Deprecated since 0.2.0" badge, the note, and the
replacement as text. The existing HTML escaping runs before the Markdown
subset, so a note cannot inject markup. `seal semver-check` keeps its current
classification: removing a deprecated item is breaking until the edition that
removes it, at which point the release note and migration make the break
explicit.

## Drawbacks

- A new manifest key and a plan schema version add compatibility surface.
- A contextual `deprecated` modifier is less familiar than `#[deprecated]`.
- Named lint levels add state to `DiagCtxt` and a new driver flag pair.
- The first edition has no behavior change, so users cannot observe an edition
  difference yet. The infrastructure is real, but the payoff arrives with the
  first breaking change.
- Publishing an edition name is a promise about future migration. Reusing a
  year for a different meaning later would be confusing.

## Rationale and alternatives

### Year-named editions instead of `0.1`

The language version already says which surface this compiler implements. If
editions reused it, a compiler release and a compatibility line would share one
value, and a package could not say "I use the 2026 contract" while running a
newer compiler. Year names are stable and independent.

### Package-scoped instead of workspace-scoped

A dependency must keep the edition it was written against. A workspace-wide
edition would silently re-edition dependencies and make a mixed graph
impossible to express.

### Contextual modifier instead of `#[deprecated]`

The lexer has no `#` token, and a general attribute grammar needs placement
rules, a value grammar, lint attributes, and codegen attributes. Building it
now would duplicate the later attributes RFC. A contextual modifier is the
established Seal mechanism for declaration metadata and keeps `deprecated`
usable as a name.

### Warning instead of a hard error

A deprecation that fails the build is a removal with extra steps. A warning
lets users migrate on their schedule while the edition policy guarantees that
the removal is announced. `--deny deprecated` is available for a project that
wants the strict behavior.

### Do nothing

The status quo silently accepts an unimplemented language version, has no way
to announce an API removal, and cannot deny one warning without denying all of
them. That is not a stable base for dependencies or a registry.

## Prior art

Rust editions separate compatibility lines from compiler releases and allow
crates to keep different editions. Rust's `#[deprecated(since, note)]` and
`allow`/`deny`/`warn` lint levels are the direct model for the diagnostic
behavior. C and C++ use deprecation attributes with tool-specific warnings;
they do not solve the cross-package compatibility problem as cleanly. Go's
`// Deprecated:` convention is documentation-only and cannot be enforced by the
toolchain.

Seal differs in two ways: deprecation is a declaration modifier rather than an
attribute, and lint levels are introduced only for the warnings that exist.
Both are deliberate scope choices.

## Unresolved questions

None blocking. The following are intentionally left to later RFCs:

- the first breaking change that creates edition `2029` or later;
- general-purpose attributes and their lint attributes;
- per-scope lint attributes inside a module or function;
- `seal fix` codemods driven by `replacement`;
- module-level deprecation.

## Edition and deprecation impact

- Edition `2026` is the baseline and introduces no behavior change.
- `project.language = "0.2"` changes from silently accepted to `E8005`; no
  compiler or standard library implements `0.2`.
- The deprecation modifier and warning are additive; no existing program
  changes meaning.
- Removal of a deprecated item requires a future accepted RFC and an edition
  boundary.

## Compatibility and migration

- Existing manifests without `edition` keep working with edition `2026`.
- Existing `seal lint` scripts using `unused_import` or `warnings` keep their
  behavior.
- Version 2 compile plans load and are upgraded to edition `2026` in memory.
- A project that wrote `project.language = "0.2"` must change it to `0.1` or
  wait for a compiler that implements `0.2`. The old value was ignored; the
  new error makes the mismatch visible.
- No public item is removed, renamed, or given a different ABI.
- Rollback is a normal revert: the edition field is optional and deprecation
  is metadata, so removing the change does not invalidate source or plans.

## Security and safety

- Manifest editions, deprecation strings, lint names, and plan fields are
  untrusted input. They are length-limited, control-character checked, and
  parsed with explicit errors.
- Deprecation notes are escaped by the documentation renderer. No raw HTML path
  is added.
- No `unsafe` is introduced. No filesystem, network, or process boundary is
  added.
- Lint policy is per-session state; two sessions remain independent.

## Testing and acceptance

The implementation is accepted when:

- edition parsing, defaulting, manifest validation, plan v3 round-trip, and v2
  upgrade pass;
- a multi-package plan keeps one edition per package;
- the deprecation modifier parses on every supported declaration and is
  rejected on every unsupported position;
- every use form warns exactly once, while self-uses and `cfg`-inactive uses do
  not;
- `--allow deprecated`, `--deny deprecated`, `--deny warnings`, unknown names,
  and duplicate names behave as specified;
- the API JSON and HTML expose the metadata with escaped text;
- deprecation changes no layout, ABI, constant evaluation, or emitted code;
- the full formatting, lint, build, test, serial, and reproducibility gates
  pass;
- docs, examples, release notes, and the acceptance matrix agree with the
  behavior.

The detailed matrix and phase exits are in the implementation plan.

## Implementation plan

1. RFC process, template, this RFC, and links.
2. Edition type, manifest validation, plan v3, package plumbing, templates,
   metadata, and standard-library manifests.
3. Deprecation syntax, AST/definition metadata, API snapshot, and docs.
4. Use-site warnings, named lint levels, driver flags, and `seal lint`.
5. Documentation, example, release notes, acceptance row, and final gate.

## Future possibilities

- The first accepted breaking change adds an `EditionFeature` variant, a new
  edition, its migration, and tests. Semantic gates need no new phase API
  because `PackageData` already carries the edition.
- A syntax-changing edition adds the parser's `Edition` parameter and rejects
  the syntax before AST construction.
- General attributes can adopt `#[deprecated(...)]` while keeping the same
  `Deprecation` value and lowering path.
- Full lint levels add per-scope attributes and a registry; `deprecated` and
  `unused_import` already use that path.
- `seal fix` uses `replacement` and the use span for a codemod.
- A generated page per edition can show the language contract and the RFCs
  that changed it.
