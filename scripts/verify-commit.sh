#!/usr/bin/env bash
#
# Verifies one commit in isolation: formatting, lint, and the full test suite.
#
# The working tree must be clean, because the script checks out the commit
# under test and restores the original checkout afterwards. The restore runs on
# every exit path, including a failing check.
#
# Usage: scripts/verify-commit.sh [<commit>]

set -euo pipefail

print_usage() {
    cat >&2 <<'USAGE'
usage: verify-commit.sh [<commit>]

Checks formatting, lint, and tests for <commit> (default: HEAD) in isolation,
then restores the original checkout.
USAGE
}

case "${1:-}" in
    -h | --help)
        print_usage
        exit 0
        ;;
esac

if [ "$#" -gt 1 ]; then
    print_usage
    exit 2
fi

if ! repository_root=$(git rev-parse --show-toplevel 2>/dev/null); then
    printf 'error: not inside a git repository\n' >&2
    exit 2
fi
cd "$repository_root"

requested=${1:-HEAD}
if ! revision=$(git rev-parse --verify --quiet "${requested}^{commit}"); then
    printf 'error: unknown commit: %s\n' "$requested" >&2
    exit 2
fi

if [ -n "$(git status --porcelain)" ]; then
    printf 'error: the working tree must be clean to verify %s\n' "$revision" >&2
    exit 2
fi

# A branch is restored by name; a detached HEAD is restored by commit.
original=$(git rev-parse --abbrev-ref HEAD)
if [ "$original" = HEAD ]; then
    original=$(git rev-parse HEAD)
fi

restore() {
    if ! git checkout --quiet "$original"; then
        printf 'warning: could not restore %s\n' "$original" >&2
    fi
}
trap restore EXIT

git checkout --quiet --detach "$revision"
printf 'verifying %s\n' "$(git log -1 --format='%h %s')"

run_step() {
    local name=$1
    shift
    printf '\n==> %s\n' "$name"
    "$@"
}

run_step 'formatting (cargo fmt --all --check)' cargo fmt --all --check
run_step 'lint (cargo clippy --workspace --all-targets -- -D warnings)' \
    cargo clippy --workspace --all-targets -- -D warnings
run_step 'tests (cargo test --workspace)' cargo test --workspace

printf '\n%s passes in isolation\n' "$revision"
