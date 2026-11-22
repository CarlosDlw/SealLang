# Workspace

The root manifest declares two members through the `crates/*` glob and makes
`crates/app` the default member. `app` depends on `greet` by path.

```console
$ cp -R examples/workspace /tmp/workspace
$ seal build --project /tmp/workspace          # builds crates/app
$ seal build --workspace --project /tmp/workspace
$ seal run --project /tmp/workspace
$ echo $?
42
```

`greet` exposes only its root through `[package.api] expose = ["root"]`, so
`use greet::answer;` works while private items stay unreachable. Selecting
`app` still compiles its dependency chain; the dependency is not added as a
top-level job.
