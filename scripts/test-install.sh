#!/usr/bin/env bash
#
# Exercises install.sh against temporary prefixes and staging roots.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd -P)
INSTALLER=$ROOT/install.sh
WORK=$(mktemp -d "${TMPDIR:-/tmp}/seal-install-test.XXXXXX")

cleanup() {
    chmod -R u+w -- "$WORK" 2>/dev/null || true
    rm -rf -- "$WORK"
}
trap cleanup EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

pass() {
    printf 'ok - %s\n' "$*"
}

info() {
    printf '\n==> %s\n' "$*"
}

assert_file() {
    [[ -f $1 ]] || fail "missing file: $1"
}

assert_exec() {
    [[ -x $1 ]] || fail "missing executable: $1"
}

descriptor_language() {
    awk -F= '
        /^[[:space:]]*language[[:space:]]*=/ {
            value = $2
            sub(/^[[:space:]]*"/, "", value)
            sub(/".*$/, "", value)
            print value
            exit
        }
    ' "$ROOT/stdlib/stdlib.toml"
}

assert_layout() {
    local prefix=$1
    assert_exec "$prefix/bin/seal"
    assert_exec "$prefix/bin/sealc"
    assert_file "$prefix/lib/seal/stdlib/$LANGUAGE/stdlib.toml"
    assert_file "$prefix/lib/seal/stdlib/$LANGUAGE/core/src/option.sl"
    assert_file "$prefix/lib/seal/stdlib/$LANGUAGE/core/src/result.sl"
    assert_file "$prefix/lib/seal/stdlib/$LANGUAGE/std/src/prelude.sl"

    if [[ -n $(find "$prefix/lib/seal/stdlib/$LANGUAGE" \
        \( -name target -o -name '*.o' -o -name '*.rlib' -o -name seal.lock \) \
        -print -quit) ]]; then
        fail "build artifacts were installed under $prefix/lib/seal/stdlib/$LANGUAGE"
    fi
}

make_project() {
    local project=$1
    mkdir -p -- "$project/src"
    cat >"$project/seal.toml" <<'EOF'
[project]
name = "install-test"
language = "0.1"

[package]
name = "install-test"
version = "0.1.0"
EOF
    cat >"$project/src/main.sl" <<'EOF'
func main() -> i32 {
  let some = Option::Some(40);
  let ok = Result::<i32, i32>::Ok(2);
  let value = match some {
    Option::Some(inner) => inner,
    Option::None => 0,
  };
  let result = match ok {
    Result::Ok(inner) => inner,
    Result::Err(_) => 0,
  };
  value + result - 42
}
EOF
}

run_without_stdlib() {
    (
        unset SEAL_STDLIB
        "$@"
    )
}

verify_installed_project() {
    local prefix=$1
    local project=$2
    if ! run_without_stdlib "$prefix/bin/seal" stdlib verify >"$WORK/verify.log" 2>&1; then
        cat "$WORK/verify.log" >&2
        fail "the installed binary cannot verify its standard library"
    fi
    if ! run_without_stdlib "$prefix/bin/seal" run --project "$project" \
        >"$WORK/project.log" 2>&1; then
        cat "$WORK/project.log" >&2
        fail "the installed binary cannot run a project without SEAL_STDLIB"
    fi
}

LANGUAGE=$(descriptor_language)
[[ -n $LANGUAGE ]] || fail "the descriptor has no language version"

info "checking the installer interface"
[[ -x $INSTALLER ]] || fail "install.sh is not executable"
bash -n "$INSTALLER"
bash -n "$ROOT/scripts/test-install.sh"
if command -v shellcheck >/dev/null 2>&1; then
    shellcheck "$INSTALLER" "$ROOT/scripts/test-install.sh"
    pass "shellcheck"
else
    printf 'skip - shellcheck is not installed\n'
fi
pass "bash syntax and executable bits"

info "building the release binaries once"
(
    cd -- "$ROOT"
    cargo build --release --locked --bin seal --bin sealc
)

info "checking --dry-run"
dry_prefix=$WORK/dry-prefix
dry_output=$("$INSTALLER" --prefix "$dry_prefix" --dry-run)
[[ ! -e $dry_prefix ]] || fail "--dry-run created the prefix"
[[ $dry_output == *"dry run: no files were written"* ]] \
    || fail "--dry-run did not report that it wrote nothing"
[[ $dry_output == *"$dry_prefix/bin/seal"* ]] \
    || fail "--dry-run did not list the binary destination"
[[ $dry_output == *"$dry_prefix/lib/seal/stdlib/$LANGUAGE"* ]] \
    || fail "--dry-run did not list the standard-library destination"
pass "--dry-run writes nothing and lists the plan"

info "checking --no-build without release binaries"
fake_repo=$WORK/fake-repo
mkdir -p -- "$fake_repo"
cp -- "$INSTALLER" "$fake_repo/install.sh"
cp -R -- "$ROOT/stdlib" "$fake_repo/stdlib"
no_build_prefix=$WORK/no-build-prefix
if bash "$fake_repo/install.sh" --prefix "$no_build_prefix" --no-build \
    >"$WORK/no-build.log" 2>&1; then
    fail "--no-build succeeded without release binaries"
fi
grep -F -- "--no-build requires target/release/seal" "$WORK/no-build.log" >/dev/null \
    || fail "--no-build failed without a clear instruction"
[[ ! -e $no_build_prefix ]] || fail "--no-build left a partial prefix"
pass "--no-build fails clearly without release binaries"

info "installing into a temporary prefix"
prefix=$WORK/prefix
if ! "$INSTALLER" --prefix "$prefix" >"$WORK/install.log" 2>&1; then
    cat "$WORK/install.log" >&2
    fail "the default install failed"
fi
assert_layout "$prefix"
make_project "$WORK/project"
verify_installed_project "$prefix" "$WORK/project"
pass "default install, layout, verify, and prelude project"

info "checking idempotence"
before=$(find "$prefix" -mindepth 1 -printf '%P\n' | sort)
if ! "$INSTALLER" --prefix "$prefix" --no-build >"$WORK/reinstall.log" 2>&1; then
    cat "$WORK/reinstall.log" >&2
    fail "the second install failed"
fi
after=$(find "$prefix" -mindepth 1 -printf '%P\n' | sort)
[[ $before == "$after" ]] || fail "a repeated install changed the prefix contents"
[[ -z $(find "$prefix" -name '.seal-install.*' -print -quit) ]] \
    || fail "a staging directory was left behind"
stdlib_dirs=$(find "$prefix/lib/seal/stdlib" -mindepth 1 -maxdepth 1 -type d | wc -l)
[[ $stdlib_dirs -eq 1 ]] || fail "the prefix contains more than one stdlib version"
bin_count=$(find "$prefix/bin" -mindepth 1 -maxdepth 1 | wc -l)
[[ $bin_count -eq 2 ]] || fail "the prefix bin directory has unexpected entries"
pass "a repeated install is idempotent"

info "installing into a prefix with spaces"
space_prefix="$WORK/prefix with spaces"
if ! "$INSTALLER" --prefix "$space_prefix" --no-build >"$WORK/space.log" 2>&1; then
    cat "$WORK/space.log" >&2
    fail "the prefix with spaces failed"
fi
assert_layout "$space_prefix"
make_project "$WORK/space-project"
verify_installed_project "$space_prefix" "$WORK/space-project"
pass "a prefix with spaces works"

info "checking the PREFIX environment variable"
env_prefix=$WORK/env-prefix
if ! PREFIX="$env_prefix" "$INSTALLER" --no-build >"$WORK/env-prefix.log" 2>&1; then
    cat "$WORK/env-prefix.log" >&2
    fail "the PREFIX environment variable was ignored"
fi
assert_layout "$env_prefix"
pass "PREFIX selects the default prefix"

info "checking a prefix without write permission"
if [[ $(id -u) -eq 0 ]]; then
    printf 'skip - running as root bypasses directory permissions\n'
else
    readonly_root=$WORK/readonly
    mkdir -p -- "$readonly_root"
    chmod 0555 -- "$readonly_root"
    if "$INSTALLER" --prefix "$readonly_root/prefix" --no-build \
        >"$WORK/readonly.log" 2>&1; then
        fail "the installer wrote into a read-only parent"
    fi
    [[ ! -e $readonly_root/prefix ]] || fail "the failed install left a partial prefix"
    chmod 0755 -- "$readonly_root"
    pass "an unwritable prefix fails without a partial install"
fi

info "checking DESTDIR staging"
destdir=$WORK/destdir
if ! DESTDIR="$destdir" "$INSTALLER" --prefix /usr/local --no-build \
    >"$WORK/destdir.log" 2>&1; then
    cat "$WORK/destdir.log" >&2
    fail "the DESTDIR install failed"
fi
staged_prefix=$destdir/usr/local
assert_layout "$staged_prefix"
make_project "$WORK/destdir-project"
if ! SEAL_STDLIB="$staged_prefix/lib/seal/stdlib/$LANGUAGE" \
    "$staged_prefix/bin/seal" stdlib verify >"$WORK/destdir-verify.log" 2>&1; then
    cat "$WORK/destdir-verify.log" >&2
    fail "the staged binary cannot verify the staged standard library"
fi
if ! SEAL_STDLIB="$staged_prefix/lib/seal/stdlib/$LANGUAGE" \
    "$staged_prefix/bin/seal" run --project "$WORK/destdir-project" \
    >"$WORK/destdir-project.log" 2>&1; then
    cat "$WORK/destdir-project.log" >&2
    fail "the staged binary cannot run a project with the staged standard library"
fi
pass "DESTDIR installs and runs from the staging root"

info "checking that profiles and PATH files are untouched"
home=$WORK/home
mkdir -p -- "$home"
cat >"$home/.bashrc" <<'EOF'
export PATH="$HOME/bin:$PATH"
EOF
printf 'export EDITOR=vim\n' >"$home/.profile"
printf 'set -g fish_greeting ""\n' >"$home/.config.fish"
before=$(cd -- "$home" && find . -type f -exec sha256sum {} \; | sort)
if ! HOME="$home" "$INSTALLER" --prefix "$WORK/profile-prefix" --no-build \
    >"$WORK/profile.log" 2>&1; then
    cat "$WORK/profile.log" >&2
    fail "the install with a custom HOME failed"
fi
after=$(cd -- "$home" && find . -type f -exec sha256sum {} \; | sort)
[[ $before == "$after" ]] || fail "the installer modified a profile file"
pass "the installer does not edit profiles or PATH"

printf '\ninstaller acceptance passed\n'
