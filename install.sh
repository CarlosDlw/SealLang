#!/usr/bin/env bash
#
# Installs the Seal toolchain and its standard library into one prefix.
#
# The installer builds from this checkout, stages every file before replacing
# anything, verifies the staged toolchain, and rolls back a failed replacement.
# It never edits shell profiles or PATH and never installs outside the selected
# prefix.

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
REPO_ROOT=$SCRIPT_DIR
STDLIB_SOURCE=$REPO_ROOT/stdlib
DESCRIPTOR=$STDLIB_SOURCE/stdlib.toml

PREFIX=${PREFIX:-/usr/local}
DESTDIR=${DESTDIR:-}
BUILD=1
DRY_RUN=0

STAGING=
PROJECT=
INSTALL_ROOT=
STDLIB_ROOT=
STDLIB_DIR=
LANGUAGE=
STDLIB_BACKUP=
BIN_BACKUP=
BIN_TMP_SEAL=
BIN_TMP_SEALC=
STDLIB_INSTALLED=0
COMMITTING=0

usage() {
    cat <<'EOF'
usage: install.sh [options]

Install `seal`, `sealc`, and the standard library for one Linux prefix.
The default build is `cargo build --release --locked --bin seal --bin sealc`.

options:
  --prefix PATH   Install prefix (default: /usr/local, or $PREFIX)
  --no-build      Use existing target/release binaries instead of building
  --dry-run       Print the actions without building or writing anything
  -h, --help      Show this help

environment:
  PREFIX          Default install prefix
  DESTDIR         Staging root prepended to PREFIX (packaging only)
  SEAL_STDLIB     Ignored during verification, which proves the installed layout

The standard library is installed under
  $PREFIX/lib/seal/stdlib/<language>/
and the binaries under
  $PREFIX/bin/
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

info() {
    printf '%s\n' "$*"
}

remove_stdlib_tree() {
    case "${STDLIB_DIR:-}" in
        "$STDLIB_ROOT"/*) rm -rf -- "$STDLIB_DIR" ;;
        *) return 1 ;;
    esac
}

restore_binaries() {
    local name
    [[ -n $BIN_BACKUP && -d $BIN_BACKUP ]] || return 0
    for name in seal sealc; do
        if [[ -e $BIN_BACKUP/$name ]]; then
            cp -p -- "$BIN_BACKUP/$name" "$INSTALL_ROOT/bin/$name" || return 1
        else
            rm -f -- "$INSTALL_ROOT/bin/$name" || return 1
        fi
    done
    return 0
}

cleanup() {
    if (( COMMITTING )); then
        if ! restore_binaries; then
            printf 'warning: could not restore the previous binaries; backup: %s\n' \
                "$BIN_BACKUP" >&2
        elif [[ -n $BIN_BACKUP ]]; then
            rm -rf -- "$BIN_BACKUP" || true
            BIN_BACKUP=
        fi
        if [[ -n $STDLIB_BACKUP && -e $STDLIB_BACKUP ]]; then
            if remove_stdlib_tree && mv -- "$STDLIB_BACKUP" "$STDLIB_DIR"; then
                STDLIB_BACKUP=
            else
                printf 'warning: the previous standard library is at %s\n' \
                    "$STDLIB_BACKUP" >&2
            fi
        elif (( STDLIB_INSTALLED )); then
            remove_stdlib_tree || true
            STDLIB_INSTALLED=0
        fi
        COMMITTING=0
    fi
    [[ -z $BIN_TMP_SEAL ]] || rm -f -- "$BIN_TMP_SEAL" || true
    [[ -z $BIN_TMP_SEALC ]] || rm -f -- "$BIN_TMP_SEALC" || true
    [[ -z $PROJECT ]] || rm -rf -- "$PROJECT" || true
    [[ -z $STAGING ]] || rm -rf -- "$STAGING" || true
    return 0
}
trap cleanup EXIT

parse_descriptor_language() {
    [[ -f $DESCRIPTOR ]] || die "standard-library descriptor not found: $DESCRIPTOR"
    LANGUAGE=$(awk -F= '
        /^[[:space:]]*language[[:space:]]*=/ {
            value = $2
            sub(/^[[:space:]]*"/, "", value)
            sub(/".*$/, "", value)
            print value
            exit
        }
    ' "$DESCRIPTOR")
    [[ -n $LANGUAGE ]] || die "the descriptor does not declare a language version"
    [[ $LANGUAGE =~ ^[0-9]+(\.[0-9]+)*$ ]] \
        || die "unsafe language version in the descriptor: $LANGUAGE"
}

parse_arguments() {
    while (( $# > 0 )); do
        case $1 in
            -h|--help)
                usage
                exit 0
                ;;
            --prefix)
                (( $# >= 2 )) || die "--prefix requires a path"
                PREFIX=$2
                shift 2
                ;;
            --prefix=*)
                PREFIX=${1#*=}
                shift
                ;;
            --no-build)
                BUILD=0
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            --)
                shift
                break
                ;;
            *)
                die "unknown option: $1 (try --help)"
                ;;
        esac
    done
    (( $# == 0 )) || die "unexpected argument: $1"
}

normalize_paths() {
    [[ -n $PREFIX ]] || die "the install prefix must not be empty"
    [[ $PREFIX == /* ]] || die "the install prefix must be absolute: $PREFIX"
    while [[ $PREFIX == */ && $PREFIX != / ]]; do
        PREFIX=${PREFIX%/}
    done
    [[ $PREFIX != / ]] || die "refusing to install directly into /; choose a prefix"

    if [[ -n $DESTDIR ]]; then
        [[ $DESTDIR == /* ]] || die "DESTDIR must be absolute: $DESTDIR"
        while [[ $DESTDIR == */ && $DESTDIR != / ]]; do
            DESTDIR=${DESTDIR%/}
        done
        [[ $DESTDIR != / ]] || DESTDIR=
    fi

    INSTALL_ROOT="${DESTDIR}${PREFIX}"
    STDLIB_ROOT="$INSTALL_ROOT/lib/seal/stdlib"
    STDLIB_DIR="$STDLIB_ROOT/$LANGUAGE"
    STDLIB_BACKUP="$STDLIB_ROOT/.${LANGUAGE}.previous.$$"
}

require_linux() {
    local system
    system=$(uname -s)
    [[ $system == Linux ]] \
        || die "this installer supports Linux only (detected: $system)"
}

print_dry_run() {
    info "dry run: no files were written"
    info ""
    if (( BUILD )); then
        info "  build:    cargo build --release --locked --bin seal --bin sealc"
    else
        info "  build:    skipped by --no-build"
    fi
    info "  install:  $REPO_ROOT/target/release/seal -> $INSTALL_ROOT/bin/seal"
    info "  install:  $REPO_ROOT/target/release/sealc -> $INSTALL_ROOT/bin/sealc"
    info "  stdlib:   $STDLIB_SOURCE -> $STDLIB_DIR"
    info "  verify:   $INSTALL_ROOT/bin/seal --version"
    info "  verify:   $INSTALL_ROOT/bin/seal stdlib verify"
    info "  verify:   temporary project with Option/Result"
    info "  profile:  no PATH or shell profile changes"
}

build_toolchain() {
    if (( BUILD )); then
        command -v cargo >/dev/null 2>&1 \
            || die "cargo was not found; install the Rust toolchain or use --no-build"
        info "Building release binaries..."
        (
            cd -- "$REPO_ROOT"
            cargo build --release --locked --bin seal --bin sealc
        )
    else
        local missing=0
        [[ -x $REPO_ROOT/target/release/seal ]] || missing=1
        [[ -x $REPO_ROOT/target/release/sealc ]] || missing=1
        if (( missing )); then
            die "--no-build requires target/release/seal and target/release/sealc; run: cargo build --release --locked --bin seal --bin sealc"
        fi
    fi
}

prepare_install_root() {
    if [[ -e $INSTALL_ROOT && ! -d $INSTALL_ROOT ]]; then
        die "the install prefix is not a directory: $INSTALL_ROOT"
    fi
    if [[ ! -d $INSTALL_ROOT ]]; then
        mkdir -p -- "$INSTALL_ROOT" \
            || die "cannot create the install prefix: $INSTALL_ROOT"
    fi
    [[ -w $INSTALL_ROOT && -x $INSTALL_ROOT ]] \
        || die "the install prefix is not writable: $INSTALL_ROOT"

    local path
    for path in "$INSTALL_ROOT/bin" "$STDLIB_ROOT"; do
        if [[ -e $path && ! -d $path ]]; then
            die "the install path is not a directory: $path"
        fi
    done

    local artifact
    for artifact in target seal seal.lock; do
        [[ ! -e $STDLIB_SOURCE/$artifact ]] \
            || die "refusing to install build artifact: $STDLIB_SOURCE/$artifact"
    done
}

stage_installation() {
    STAGING=$(mktemp -d "$INSTALL_ROOT/.seal-install.XXXXXX") \
        || die "cannot create a staging directory in $INSTALL_ROOT"
    mkdir -p -- "$STAGING/bin" "$STAGING/lib/seal/stdlib"

    install -m 0755 -- "$REPO_ROOT/target/release/seal" "$STAGING/bin/seal"
    install -m 0755 -- "$REPO_ROOT/target/release/sealc" "$STAGING/bin/sealc"
    cp -R -- "$STDLIB_SOURCE" "$STAGING/lib/seal/stdlib/$LANGUAGE"

    [[ -f $STAGING/lib/seal/stdlib/$LANGUAGE/stdlib.toml ]] \
        || die "the staged standard library has no descriptor"

    info "Verifying the staged toolchain..."
    SEAL_STDLIB="$STAGING/lib/seal/stdlib/$LANGUAGE" \
        "$STAGING/bin/seal" --version >/dev/null
    SEAL_STDLIB="$STAGING/lib/seal/stdlib/$LANGUAGE" \
        "$STAGING/bin/seal" stdlib verify >/dev/null
}

commit_installation() {
    local bin_dir="$INSTALL_ROOT/bin"
    local name

    mkdir -p -- "$bin_dir" "$STDLIB_ROOT"
    BIN_BACKUP="$INSTALL_ROOT/.seal-previous-bin.$$"
    mkdir -p -- "$BIN_BACKUP"
    for name in seal sealc; do
        if [[ -e $bin_dir/$name || -L $bin_dir/$name ]]; then
            cp -p -- "$bin_dir/$name" "$BIN_BACKUP/$name"
        fi
    done

    COMMITTING=1
    if [[ -e $STDLIB_DIR || -L $STDLIB_DIR ]]; then
        mv -- "$STDLIB_DIR" "$STDLIB_BACKUP"
    fi

    BIN_TMP_SEAL="$bin_dir/.seal.new.$$"
    BIN_TMP_SEALC="$bin_dir/.sealc.new.$$"
    install -m 0755 -- "$STAGING/bin/seal" "$BIN_TMP_SEAL"
    install -m 0755 -- "$STAGING/bin/sealc" "$BIN_TMP_SEALC"
    mv -f -- "$BIN_TMP_SEAL" "$bin_dir/seal"
    BIN_TMP_SEAL=
    mv -f -- "$BIN_TMP_SEALC" "$bin_dir/sealc"
    BIN_TMP_SEALC=

    mv -- "$STAGING/lib/seal/stdlib/$LANGUAGE" "$STDLIB_DIR"
    STDLIB_INSTALLED=1
    COMMITTING=0

    rm -rf -- "$BIN_BACKUP" || true
    BIN_BACKUP=
    rm -rf -- "$STDLIB_BACKUP" || true
    STDLIB_BACKUP=
}

run_installed() {
    if [[ -n $DESTDIR ]]; then
        SEAL_STDLIB="$STDLIB_DIR" "$@"
    else
        (
            unset SEAL_STDLIB
            "$@"
        )
    fi
}

verify_project() {
    local binary=$1
    PROJECT=$(mktemp -d "${TMPDIR:-/tmp}/seal-install-check.XXXXXX") \
        || die "cannot create a temporary project"
    mkdir -p -- "$PROJECT/src"
    cat >"$PROJECT/seal.toml" <<'EOF'
[project]
name = "install-check"
language = "0.1"

[package]
name = "install-check"
version = "0.1.0"
EOF
    cat >"$PROJECT/src/main.sl" <<'EOF'
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
    if ! run_installed "$binary" run --project "$PROJECT"; then
        die "the installed toolchain cannot run a project with the standard-library prelude"
    fi
    rm -rf -- "$PROJECT"
    PROJECT=
}

verify_installed() {
    local binary="$INSTALL_ROOT/bin/seal"
    info "Verifying the installed toolchain..."
    run_installed "$binary" --version >/dev/null
    run_installed "$binary" stdlib verify >/dev/null
    verify_project "$binary"
}

main() {
    parse_arguments "$@"
    require_linux
    parse_descriptor_language
    normalize_paths

    if (( DRY_RUN )); then
        print_dry_run
        exit 0
    fi

    build_toolchain
    prepare_install_root
    stage_installation
    commit_installation
    verify_installed

    info ""
    info "Installed Seal into $INSTALL_ROOT"
    info "  binary:  $INSTALL_ROOT/bin/seal"
    info "  stdlib:  $STDLIB_DIR"
    if [[ -z $DESTDIR ]]; then
        info ""
        info "Add $INSTALL_ROOT/bin to PATH if it is not there already."
    fi
    info "No shell profile or PATH file was modified."
}

main "$@"
