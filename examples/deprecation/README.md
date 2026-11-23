# Deprecation

This package shows the deprecation lifecycle without removing the old
declaration. `old` stays callable, carries `since`, `note`, and `replacement`
metadata, and every use warns with the stable `deprecated` lint.

```console
$ cp -R examples/deprecation /tmp/deprecation
$ seal run --project /tmp/deprecation
warning[deprecated]: `old` is deprecated since 0.1.0
...
$ echo $?
42
```

The warning is expected and does not change the exit status. Use
`seal lint --project /tmp/deprecation --allow deprecated` to suppress it, or
`--deny deprecated` to make the call fail the build. The metadata also appears
in `seal api` and `seal doc`.

`old` is still declared, so this example is not a migration: removal happens
only in an edition named by an accepted RFC, with a release note and a migration
recipe.
