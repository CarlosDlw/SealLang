# Features and `cfg`

`left` and `right` both depend on `common`, but each edge requests a different
feature. Feature sets are additive inside one instance and are never unified
across edges, so the build creates two instances of `common`:

- the `left` edge sees `cfg(feature("a"))` and returns `1`;
- the `right` edge sees `cfg(feature("b"))` and returns `2`.

```console
$ cp -R examples/features /tmp/features
$ seal build --project /tmp/features
$ seal tree --project /tmp/features
$ seal why --features --project /tmp/features
$ seal run --project /tmp/features
$ echo $?
3
```

Inactive `cfg` items are removed before type checking, so both `which`
declarations may keep the same name. The two instances also receive distinct
symbol names; neither can collide at link time.
