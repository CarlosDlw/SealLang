# Dependencies and locking

A Seal dependency is either a path inside the project or a package from a
configured registry. Both kinds share the same manifest syntax, graph, feature
resolution, and `seal.lock` format.

```toml
[dependencies]
geometry = { path = "../geometry", version = "0.1" }
foo = "1.2"
bar = { version = "^3", package = "bar-package", features = ["std"] }
windows = { version = "0.9", when = { os = "windows" } }
```

The key is the alias used by imports. A string is shorthand for the same
registry package name; `package` selects a different published name. A
dependency declares either `path` or `version`, never both and never `git`. An
inactive `when` condition does not hide the requirement from the lockfile, but
it keeps the edge out of the feature graph for that target.

## Path dependencies

A path must stay inside the project root. The target package must have a
`[package]` table, and an optional version must match its declared version.
Escapes, missing manifests, and cycles are reported while the project loads
(`E8021`–`E8023`, `E8006`).

## Registry dependencies

A registry dependency needs a trusted index:

```toml
[registry]
url = "https://registry.example.com/"
tuf-url = "https://registry.example.com/tuf/"
tuf-root = "trusted/root.json"
mirror = "https://mirror.example.com/"
token-env = "SEAL_REGISTRY_TOKEN"
# credentials = "seal/credentials.toml"
# offline = true
# allow-insecure = true

[registry.policy]
allow-licenses = ["MIT", "Apache-2.0"]
deny-licenses = ["GPL-3.0"]
allow-sources = ["registry+https://registry.example.com/"]
require-provenance = true
```

`url` is the sparse-index base. Index documents are resolved from the sparse
layout (`index/3/f/foo.json`, `index/@scope/name.json`) and every document and
archive is verified against TUF before use. `tuf-root` is required whenever a
registry is configured: it is the locally pinned trust anchor. `tuf-url`
defaults to `url`; `SEAL_REGISTRY_URL`, `SEAL_REGISTRY_MIRROR`,
`SEAL_REGISTRY_TUF_URL`, and `SEAL_REGISTRY_TUF_ROOT` override the manifest.
HTTPS is required unless `allow-insecure = true` is set for a local test
registry. Embedded credentials in URLs are rejected.

The token comes from `token-env` (default `SEAL_REGISTRY_TOKEN`) or from
`credentials`, `$SEAL_CREDENTIALS`, or the user config path. A credentials file
must not be group- or world-readable; tokens are never logged or shown in
errors. A registry supporting private packages therefore needs only a token,
not a URL secret.

## Resolution

Versions follow semantic-version ranges: `1.2` and `^1.2` accept
`>=1.2.0, <2.0.0`, `~1.2` accepts `>=1.2.0, <1.3.0`, and `=`, `>=`, `<`, and
`*` keep their usual meaning. Resolution uses PubGrub and reports the path that
caused a conflict. Prereleases are selected only when a requirement explicitly
names one. Yanked versions are skipped unless the lockfile already pins one;
an explicit exact request for a yanked version is `E8033`.

Two major versions of a package may coexist, as may two `0.minor` lines. They
are separate PubGrub packages, separate lockfile sources, and separate build
instances. Same-major requirements must intersect and resolve to exactly one
version. `seal tree --duplicates` lists every package with several selected
versions.

`--minimal-versions` prefers the lowest compatible version instead of the
newest; `--latest` selects the default newest-compatible behavior explicitly
and cannot be combined with it. Locked versions are preferred when they still
satisfy every edge.
`--precise package@version` pins one package for `seal update`; the pin is
subject to the manifest range, so widening a requirement is an explicit
manifest edit.

## Commands

```console
$ seal add foo@1.2 --yes
$ seal add @scope/foo@^2 --features std --dry-run
$ seal remove foo --yes
$ seal update
$ seal update foo --precise foo@1.2.0
$ seal vendor
$ seal vendor --clean
```

`add` and `remove` edit the selected package's manifest with `toml_edit`, then
load the project, resolve, write the lockfile, and run `seal check`. They do
not write anything without `--yes`; `--dry-run` prints the proposed graph and
leaves the manifest and lockfile untouched. A failed check rolls the edit and
the lockfile back. `update` rewrites only the lockfile; `--dry-run` prints the
diff without writing. A bare `update` refreshes every registry edge; naming
packages refreshes only those. `--minimal-versions` applies to resolution, and
`--offline` forbids every network request.

`vendor` copies every locked registry package into `vendor/<name>-<version>`,
verifies its checksum and tree digest, and writes `vendor/seal-vendor.toml`.
`seal build --offline` uses that tree without contacting the registry. The
global `--vendor` flag forces the same path even when the network is allowed.
Remove the vendor tree with `--clean` or delete it together with
`seal-vendor.toml`.

## Lockfile v2

`seal.lock` is deterministic and contains:

- `version = 2`, the compiler `toolchain`, and resolved `targets`;
- one `[[package]]` per name, version, and source with `checksum`, optional
  `api`, the union of `features`, resolved `cfg`, optional `provenance`, and
  `yanked`;
- exact `[[package.dependencies]]` edges with alias, name, version, source, and
  features;
- concrete `[[instances]]` with their instance hash and cfg;
- resolved `[[native]]` records.

A v1 lockfile is read and upgraded in memory to v2; the next write stores the
new shape. `--locked` fails with `E8008` when resolution would change the
lockfile. `--frozen` also forbids network access. A missing checksum marker or
a mismatch is `E8021`; a path dependency that leaves the project is also an
integrity failure.

## Audit and SBOM

```console
$ seal audit
$ seal audit --json
$ seal deny --json
$ seal sbom --output sbom.cdx.json
```

`audit` checks locked registry packages against the signed advisory feed,
license policy, allowed sources, yanked releases, checksums, and required
provenance. Findings are printed with stable codes and the command exits `1`
when any finding remains. `--json` emits a machine-readable report; `deny` is
an alias. `--offline` reads the cached registry data.

`sbom` emits CycloneDX 1.5 JSON from the resolved graph. Every component carries
its source, SHA-256 digest, enabled features, compiled targets, and license when
declared; the document also records the project and toolchain. The serial
number is derived from the project and component digests, so an unchanged graph
produces the same document.

## Diagnostics

| Code | Meaning |
|------|---------|
| `E8020` | Registry configuration or TUF trust failure |
| `E8021` | Dependency integrity check failed |
| `E8022` | Missing path dependency |
| `E8023` | Incompatible dependency version |
| `E8024` | Invalid dependency declaration or CLI edit |
| `E8025` | Registry index or transport failure |
| `E8026` | Version resolution failure |
| `E8027` | Package archive or manifest rejected |
| `E8028` | Registry lock conflict |
| `E8029` | Invalid vendor tree |
| `E8030` | Dependency audit failed |
| `E8031` | SBOM generation failed |
| `E8032` | Registry query failed |
| `E8033` | Yanked version selected |
| `E8034` | License not allowed |
| `E8035` | Source not allowed |
| `E8036` | Advisory affects a locked version |
| `E8037` | Provenance missing or invalid |
| `E8038` | Registry response is too large |
| `E8039` | Inconsistent registry metadata |
| `E8080` | TUF verification failed |
| `E8081` | Registry namespace error |
| `E8082` | Published version is immutable |
| `E8083` | Registry publish rejected |
| `E8084` | Registry credentials unavailable |
| `E8085` | Offline registry data is missing |
| `E8086` | Package metadata mismatch |
| `E8087` | Provenance verification failed |
| `E8088` | Registry cache error |
| `E8089` | Corrupt registry cache entry |
