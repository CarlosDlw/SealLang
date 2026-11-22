# Freestanding

`targets.kernel` sets `runtime = "none"` and `panic = "trap"`, so the build
skips the Seal runtime archive and the C `main` wrapper, uses `_start` as the
entry symbol, and links with the declared linker script.

```console
$ cp -R examples/freestanding /tmp/freestanding
$ seal build --project /tmp/freestanding
$ seal run --project /tmp/freestanding
$ echo $?
42
```

`seal build --target <TRIPLE> --emit=obj` inspects a cross target without
running it; a cross executable needs a declared linker and sysroot
(`E8062`). The `kernel` template from `seal new --template kernel` generates
this same shape.
