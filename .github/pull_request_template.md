## Summary

What changes, and why.

## Contract impact

- [ ] No language, manifest, compile-plan, lockfile, registry, or public-API
      contract changes.
- [ ] A contract change is covered by an accepted RFC in `docs/rfcs/`:
      RFC number and link:

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo build --workspace --locked`
- [ ] `cargo test --workspace --locked`
- [ ] Documentation, examples, and release notes are updated in this change.
- [ ] No secret, build artifact, cache, or unrelated change is included.
