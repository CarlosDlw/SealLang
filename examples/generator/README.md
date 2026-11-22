# Generator

`targets.table` is a generator: it reads `data/value.txt`, writes
`generated/value.sl`, and declares both sides of that contract. `targets.app`
opts into the generated file with `generated = ["table"]`.

```console
$ cp -R examples/generator /tmp/generator
$ seal build --project /tmp/generator
$ seal run --project /tmp/generator
$ echo $?
42
```

The first build runs the generator, validates that every declared output exists
and that no undeclared file was written, stores the output in the content
cache, and compiles it as part of the package. A repeated build is a cache hit.
Changing `data/value.txt` or `src/generate.sl` reruns the generator and
recompiles the consumer.
