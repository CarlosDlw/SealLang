# Installing Seal

`install.sh` installs `seal`, `sealc`, and the standard library from this
checkout. It is a Linux installer that builds from source, stages the complete
installation, verifies it, and only then replaces the selected prefix.

The installer does not download a compiler, does not use root unless the
prefix requires it, and never edits `PATH` or a shell profile.

## Requirements

- Linux. Other operating systems are rejected with an explicit error.
- A Rust toolchain with `cargo` when the installer builds. The repository pins
  its toolchain in `rust-toolchain.toml`.
- Write permission on the selected prefix. A system prefix such as
  `/usr/local` usually needs `sudo`; a prefix under `$HOME` does not.

When `--no-build` is used, the release binaries must already exist at
`target/release/seal` and `target/release/sealc`.

## Install

For a per-user installation:

```console
$ ./install.sh --prefix "$HOME/.local"
```

For `/usr/local`, build as your user first and let `sudo` perform only the
installation:

```console
$ cargo build --release --locked --bin seal --bin sealc
$ sudo ./install.sh --no-build
```

Running `sudo ./install.sh` without `--no-build` also works when `cargo` is
available to root, but building as the invoking user avoids a toolchain that is
only installed in that user's home directory.

The prefix defaults to `/usr/local`. `PREFIX` can provide the default instead:

```console
$ PREFIX="$HOME/.local" ./install.sh
```

An explicit `--prefix` overrides `PREFIX`. Pass `--prefix` rather than relying
on environment variables through `sudo`, because `sudo` commonly resets the
environment.

### Options

| Option | Effect |
|--------|--------|
| `--prefix PATH` | Install prefix. Overrides `PREFIX`; defaults to `/usr/local`. |
| `--no-build` | Use existing `target/release/seal` and `target/release/sealc`. |
| `--dry-run` | Print the build, paths, and verification steps without writing anything. |
| `-h`, `--help` | Print usage. |

### Layout

The installer writes:

```text
<prefix>/bin/seal
<prefix>/bin/sealc
<prefix>/lib/seal/stdlib/<language>/stdlib.toml
<prefix>/lib/seal/stdlib/<language>/core/
<prefix>/lib/seal/stdlib/<language>/std/
```

`<language>` comes from `stdlib/stdlib.toml` and is validated before it is used
as a path component. The current value is `0.1`.

The installed `seal` and `sealc` find the standard library relative to their own
location through `<prefix>/lib/seal/stdlib/<language>`. No environment variable
is required for a normal installation. Files outside `<prefix>/lib/seal/stdlib`
are not touched.

## Verification

After staging, the installer runs the staged binaries against the staged
standard library. After installation it runs, with `SEAL_STDLIB` removed:

```console
$ <prefix>/bin/seal --version
$ <prefix>/bin/seal stdlib verify
```

It also creates a temporary project that uses `Option` and `Result` without
imports and runs it. A failure while replacing files restores the previous
installation. A failure in the final checks reports an error and exits nonzero.

The `SEAL_STDLIB` environment variable can still select another standard-library
root for development. The installer deliberately ignores it during verification
so that the installed layout itself is tested.

## PATH

Add the binary directory to `PATH` yourself if it is not already there:

```console
$ export PATH="$HOME/.local/bin:$PATH"
```

Put that line in the profile for your shell if the installation is permanent.
The installer prints the hint but does not modify shell profiles or any other
`PATH` file.

## Upgrading

Run the installer again. It stages the new files, verifies them, backs up the
current binaries and the current `<language>` standard library, and replaces
them. A repeated install with the same version is idempotent: the prefix keeps
one `seal`, one `sealc`, and one directory for the current language.

Standard libraries for other language versions are preserved. Existing files
outside the standard-library language directory are preserved as well.

## DESTDIR

`DESTDIR` prepends a staging root to `PREFIX`. It is intended for packaging, not
for a normal installation:

```console
$ DESTDIR="$PWD/pkg" ./install.sh --no-build --prefix /usr/local
```

This writes the complete layout under `$PWD/pkg/usr/local` and verifies the
staged binary with `SEAL_STDLIB` pointing at the staged standard library. Copy
that tree into the package or image; do not run it as a normal prefix unless
the staging path is the path you want the binaries to use.

## Manual uninstall

There is no uninstall command. Inspect the prefix, then remove the two binaries
and the current standard-library language directory:

```console
$ ls "$PREFIX/bin/seal" "$PREFIX/bin/sealc"
$ rm -f "$PREFIX/bin/seal" "$PREFIX/bin/sealc"
$ rm -rf "$PREFIX/lib/seal/stdlib/0.1"
```

Do not remove `$PREFIX` itself. It may contain unrelated files, and the
installer never removes it. If other language versions exist under
`$PREFIX/lib/seal/stdlib`, remove only the directories you intend to delete.

## Development checks

The installer acceptance test builds the release binaries, installs into
temporary prefixes, checks idempotence, `--dry-run`, `--no-build`, prefixes with
spaces, an unwritable prefix, `DESTDIR`, and profile preservation:

```console
$ bash -n install.sh scripts/test-install.sh
$ shellcheck install.sh scripts/test-install.sh
$ scripts/test-install.sh
```

The Linux CI job runs the same syntax, lint, and acceptance steps.

## Limitations

- Linux only. macOS and Windows are not supported by this installer.
- It builds from this checkout; it is not a network installer and does not
  bootstrap Rust.
- It does not create distribution packages, package-manager metadata, shell
  completions, or an uninstall command.
- It installs only the `seal` and `sealc` binaries; it does not install a
  linker, LLVM, or the Rust toolchain.
