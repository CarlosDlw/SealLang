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
