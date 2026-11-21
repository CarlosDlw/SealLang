# Registry protocol and trust

The registry client is deliberately small: a sparse HTTP index, a TUF metadata
chain, immutable `.tar.zst` archives, and a signed advisory feed. A package is
usable only after its metadata chain, target digest, archive digest, and package
manifest all agree.

## Index layout

The base URL is the `url` from `[registry]`. Paths follow the sparse-index
convention:

| Package | Path |
|---------|------|
| `a` | `index/1/a.json` |
| `ab` | `index/2/ab.json` |
| `abc` | `index/3/a/abc.json` |
| `serde` | `index/se/rd/serde.json` |
| `@scope/pkg` | `index/@scope/pkg.json` |
| catalog | `index/catalog.json` |
| advisories | `advisories.json` (or the configured URL) |
| archives | the `archive` path in each version entry |

An index document has one `format` (currently `1`), the exact package `name`, and
a `versions` map. Every version entry carries:

```json
{
  "checksum": "sha256:<64 lowercase hex>",
  "archive": "packages/foo/1.2.0/foo-1.2.0.tar.zst",
  "yanked": false,
  "license": "MIT",
  "description": "Example package",
  "repository": "https://example.com/foo",
  "provenance": "sha256:<64 lowercase hex>",
  "dependencies": [
    {
      "name": "bar",
      "alias": "bar",
      "req": "^1",
      "features": ["std"],
      "default-features": true,
      "optional": false,
      "when": { "os": "linux" }
    }
  ],
  "features": { "default": [], "std": [] }
}
```

`checksum` and `provenance` must be lowercase `sha256:` digests. `archive` must
be a relative path ending in `.tar.zst` and cannot contain `..` or backslashes.
Version keys must be canonical semver. Duplicate aliases and malformed feature
names are rejected. A package name is scoped (`@scope/name`) or a single safe
path component consisting of ASCII letters, digits, `-`, `_`, and `.`.

The catalog is `{"format": 1, "packages": [{"name", "latest", "description",
"license"}]}`. It powers `seal search`; it is not trusted for resolution.

## TUF chain

`[registry] tuf-root` must name a local `root.json`. The client verifies that
root with its own keys, then fetches `root.json`, `timestamp.json`,
`snapshot.json`, and `targets.json` from `tuf-url`, verifying:

- Ed25519 signatures against the key ids and threshold for each role;
- RFC 3339 expiry, including the frozen-metadata rejection;
- timestamp/snapshot/targets version rollback against the last successful
  verification in `seal/registry/tuf/state.json`;
- the length and SHA-256 digest named for `snapshot.json` and `targets.json`;
- the timestamp/snapshot/targets version cross-references (mix-and-match).

The verified targets document lists every index document, archive, catalog,
and advisory file with a length and SHA-256 digest. A target that is absent,
oversized, truncated, or has the wrong digest is rejected. The client never uses
a target before this check.

If the registry publishes a newer root, the client verifies it with both the old
and new root before trusting it. A missing trusted root, a bad signature, an
expired document, a rollback, or a mismatch is `E8080`.

## HTTP behavior

HTTPS is required by default. `file://` is supported for local fixtures and
`http://` is accepted only with `allow-insecure = true`; it is intended for a
loopback test registry. URLs with embedded credentials are rejected. The client
uses bounded redirects, a global timeout, bounded response bodies, ETag caching,
and three attempts with exponential backoff for transient failures and 429/5xx
responses. A mirror is tried before the primary URL and must use the same TUF
metadata.

Credentials come from `token-env` or a credentials file. On Unix the file must
have mode `0600`; a group- or world-readable file is an error. The token is sent
as `Authorization: Bearer ...`; debug output and errors redact it. A 401/403 is
`E8084`.

`--offline`, `--frozen`, `SEAL_OFFLINE`, and `[registry] offline = true` forbid
network access. Cached HTTP bodies, verified package directories, and the vendor
tree remain available; a missing byte is `E8085`.

## Archives

An archive is a zstd-compressed tar file. It must contain `seal.toml` at the
root. Extraction rejects absolute paths, `.`/`..`, backslashes, symlinks, hard
links, character/block devices, FIFOs, duplicate entries, more than 50,000
entries, files above 64 MiB, and a total expansion above 512 MiB. The destination
is created exclusively and removed on every error. File execute bits are
normalized to `0755` or `0644`.

After extraction, the package manifest must name the index's package name and
version, and its dependency aliases, package names, and requirements must match
the index entry exactly. A mismatch is `E8086`; a digest mismatch is `E8021` or
`E8080` depending on whether TUF or the index caught it.

## Cache and vendor

Downloads land under `seal/registry/packages/<name-hash>/<version>-<digest>`,
with a `.seal-checksum` marker. A cached directory is reused only when its marker
matches the index digest. Locked packages may also be read from
`vendor/<name>-<version>` and are verified against both the checksum marker and
the tree digest in `vendor/seal-vendor.toml`.

The vendor tree is never inferred from a directory name alone. `seal vendor`
refuses to overwrite an unrecognized non-empty `vendor/` directory, copies only
regular files, rejects symlinks and special files, and writes the manifest
atomically.

## Advisories

`advisories.json` is a TUF target with this shape:

```json
{
  "format": 1,
  "advisories": [
    {
      "id": "SEAL-2026-0001",
      "package": "foo",
      "affected": "<1.2.0",
      "severity": "high",
      "description": "An old foo release is vulnerable",
      "url": "https://example.com/advisories/SEAL-2026-0001"
    }
  ]
}
```

The feed is validated before use. `affected` is a semver requirement; every
matching locked package adds an `E8036` finding to `seal audit`. The advisory id,
package, and range are required; malformed entries are `E8030`.

## Protocol limits

These limits are intentional and are part of the client contract:

| Value | Limit |
|-------|-------|
| metadata document | 2 MiB |
| target file | 512 MiB |
| HTTP body | 512 MiB |
| archive entries | 50,000 |
| one extracted file | 64 MiB |
| total extracted bytes | 512 MiB |
| archive path | 1,024 bytes |
| resolution steps | 100,000 |

A project that needs a larger package should split it into modules or publish a
smaller archive. Disabling a limit is not a supported configuration.

## Publishing

`seal publish` builds the package from its tracked sources, writes a
reproducible `.tar.zst` (fixed mtime, uid, gid, and mode), and attaches three
signed artifacts:

- `seal.api.json`, the versioned public-API snapshot (`crates/api`);
- `seal.sbom.json`, CycloneDX 1.5 for the package and its resolved dependencies;
- a DSSE envelope containing an in-toto statement with a SLSA v1 predicate.

The provenance subject is the archive SHA-256. The predicate binds the package
name, version, repository, SBOM digest, and API digest. The publisher key id and
public Ed25519 key travel with the request; the registry recomputes the key id
and rejects a mismatch. `--identity` records an OIDC subject, and trusted
publishing rejects an identity that differs from the authenticated subject.

Publishing is immutable: a second request for the same canonical version is
`E8082`, even when the bytes are identical. The server cross-checks the archive
manifest, license, readme, `.sl` sources, and dependency table before storing
anything. Path and git dependencies cannot be published.

`--workspace` publishes in dependency order. A version already present is
skipped with a warning so an interrupted run can be repeated; every other error
stops the run and reports that earlier versions remain published. A dirty Git
tree is `E8095` unless `--allow-dirty` is given. `--dry-run` builds and prints
the file list, archive and attestation digests, and API diff without writing to
the registry; it still validates the package.

A breaking API change requires a major bump (or a minor bump on `0.x`) both in
the client and on the server. The comparison is against the newest earlier
published version that has an API snapshot. `seal api --json` prints the
snapshot and `seal semver-check` compares it with a published version or a local
file.

## Namespaces and tokens

A scoped package `@scope/name` belongs to the namespace `@scope`. An unscoped
package belongs to its own name, or to `default-namespace` when the server is
configured with one. Publishing into an unowned namespace is `E8083`.

Namespaces are claimed with `seal owner --claim --proof`:

- `config` and `org` succeed only for a preapproved namespace or an admin;
- `oidc` requires a verified OIDC token and records its subject as the owner;
- `dns` checks a TXT record `_seal.<namespace>` for the supplied token.

Owners are identities (token names or OIDC subjects). `--add`, `--remove`, and
`--list` manage them; a non-owner cannot change the set. Tokens carry explicit
`action:namespace` scopes (`publish`, `yank`, `owners`, `read`) and may be
limited to a namespace list. The server issues dynamic tokens only to an admin
and stores only their SHA-256 digest.

## Transparency log

The server maintains an RFC 6962 Merkle tree of every publish and yank. Each
leaf records the operation, package, version, archive/provenance/SBOM/API
digests, publisher key id, and time. A publication response and the index entry
carry the exact leaf, its inclusion proof, and a signed tree head (STH). The STH
is signed with the `log` key published as an extra role in the TUF root.

`seal publish` verifies the STH signature, the tree size and root, and the
inclusion path before reporting success. `seal audit --require-provenance`
repeats this for every registry package, and additionally verifies the archive
digest, DSSE signature, SBOM digest, API digest, and provenance subject. A
tampered leaf, proof, or head is `E8097`; unverifiable attestations are `E8096`.

## Reference server

`seal-registry-server` serves the sparse index, catalog, archives, TUF metadata,
advisories, and the write API from one storage directory:

```console
$ seal-registry-server --config registry.toml
$ seal-registry-server --bind 127.0.0.1:8787 --storage .seal-registry
```

| Path | Contents |
|------|----------|
| `keys.json` | five Ed25519 seeds, mode `0600` |
| `state.redb` | namespaces, versions, tokens, log, audit events, quotas |
| `blobs/<sha256>` | content-addressed archives and attestations |
| `public/index/` | sparse index and catalog |
| `public/packages/` | archive, API, SBOM, and provenance files |
| `public/tuf/` | root, targets, snapshot, timestamp |
| `public/advisories.json` | advisory feed |

Metadata updates are serialized by a mutation lock and committed in durable
redb transactions; a restart reloads the state, rebuilds missing public targets
from the CAS, and keeps the log. The server never executes package code: archives are
extracted with the bounded client extractor and only their manifest and regular
files are read.

Limits come from the configuration: `max-body`, `requests-per-minute`,
`publish-bytes-per-day`, and `max-versions-per-package`. Exceeding a quota or
rate limit is `E8098`. TLS is enabled by setting `tls-cert` and `tls-key`; both
must be present. Static tokens and OIDC issuers are configured in the same file,
and configured secrets are never logged.

Root rotation (`POST /api/v1/tuf/rotate`, admin only) generates five new seeds
and writes a root signed by both the previous and the new root key. The client
verifies that chain, replaces its trusted root atomically, and continues using
the new keys. A rollback, freeze, mix-and-match, bad signature, or missing
threshold is `E8080`.
