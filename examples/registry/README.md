# Registry consumer

This project depends on `foo = "~1.2"` from a sparse registry. The manifest pins
a trusted TUF root (`trusted-root.json`) and an allow-list of licenses; the
client verifies the root, targets, snapshot, timestamp, and every archive
checksum before extraction.

The committed manifest points at a placeholder `file://` registry because a
real registry URL and trust root are deployment-specific. The reference
acceptance test copies this directory, writes a local registry fixture, and
then:

```console
$ seal vendor --project /tmp/registry
$ seal build --offline --project /tmp/registry
$ seal run --offline --project /tmp/registry
$ echo $?
42
```

`seal vendor` copies the locked package into `vendor/<name>-<version>`, checks
its checksum and tree digest, and writes `vendor/seal-vendor.toml`. The offline
build reads only that tree; deleting the registry does not affect it.

To use a real registry, replace `[registry]` with its HTTPS URLs and commit the
pinned `root.json` as `trusted-root.json`. `seal audit` checks advisories,
licenses, yanked versions, and provenance; `seal sbom` writes CycloneDX.
