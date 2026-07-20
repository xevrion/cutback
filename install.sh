#!/bin/sh
# Installer for cutback.
#
# Builds from source and installs the binary and man page. POSIX sh on
# purpose, so it runs the same under sh, dash, bash and zsh.
#
#   ./install.sh                 install to ~/.local
#   ./install.sh --prefix /usr/local
#   ./install.sh --uninstall
#
# Reads CUTBACK_PREFIX and DESTDIR if set.

set -eu

REPO_URL="https://github.com/xevrion/cutback"
MIN_RUST="1.82"

prefix="${CUTBACK_PREFIX:-$HOME/.local}"
destdir="${DESTDIR:-}"
mode="install"
assume_yes=0

# Colour only when stdout is a terminal that supports it, so piping to a file
# or a log does not fill it with escape codes.
if [ -t 1 ] && [ "${TERM:-dumb}" != "dumb" ] && [ -z "${NO_COLOR:-}" ]; then
    bold=$(printf '\033[1m'); dim=$(printf '\033[2m')
    red=$(printf '\033[31m'); green=$(printf '\033[32m')
    yellow=$(printf '\033[33m'); reset=$(printf '\033[0m')
else
    bold=''; dim=''; red=''; green=''; yellow=''; reset=''
fi

say()  { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$bold" "$reset" "$*"; }
warn() { printf '%swarning:%s %s\n' "$yellow" "$reset" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$red" "$reset" "$*" >&2; exit 1; }

usage() {
    cat <<EOF
${bold}cutback installer${reset}

Usage:
  ./install.sh [options]

Options:
  --prefix PATH   Install under PATH (default: \$HOME/.local)
  --uninstall     Remove an installed cutback
  -y, --yes       Do not prompt
  -h, --help      Show this help

Environment:
  CUTBACK_PREFIX  Same as --prefix
  DESTDIR         Staging root, for packagers

Files installed:
  PREFIX/bin/cutback
  PREFIX/share/man/man1/cutback.1
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)     [ $# -ge 2 ] || die "--prefix needs a path"; prefix="$2"; shift 2 ;;
        --prefix=*)   prefix="${1#*=}"; shift ;;
        --uninstall)  mode="uninstall"; shift ;;
        -y|--yes)     assume_yes=1; shift ;;
        -h|--help)    usage; exit 0 ;;
        *)            die "unknown option: $1. Try --help" ;;
    esac
done

bindir="$destdir$prefix/bin"
mandir="$destdir$prefix/share/man/man1"

confirm() {
    [ "$assume_yes" -eq 1 ] && return 0
    [ -t 0 ] || return 0   # Non interactive, for example piped from curl.
    printf '%s [y/N] ' "$1"
    read -r reply </dev/tty 2>/dev/null || return 0
    case "$reply" in [yY]|[yY][eE][sS]) return 0 ;; *) return 1 ;; esac
}

if [ "$mode" = "uninstall" ]; then
    step "Removing cutback"
    removed=0
    for f in "$bindir/cutback" \
             "$mandir/cutback.1" \
             "$destdir$prefix/share/bash-completion/completions/cutback" \
             "$destdir$prefix/share/zsh/site-functions/_cutback" \
             "$destdir$prefix/share/fish/vendor_completions.d/cutback.fish"; do
        if [ -e "$f" ]; then
            rm -f "$f" && say "  removed $f" && removed=1
        fi
    done
    [ "$removed" -eq 1 ] || warn "nothing found under $prefix"
    say ""
    say "Project histories in .cutback directories were left alone."
    say "Those hold your edit history, delete them yourself if you want them gone."
    exit 0
fi

# Kdenlive is Linux first and the watcher relies on inotify, so anything else
# would build but not work. Better to say so than to install something broken.
case "$(uname -s)" in
    Linux) ;;
    *) die "cutback supports Linux only. Detected: $(uname -s)" ;;
esac

step "Checking prerequisites"

command -v cargo >/dev/null 2>&1 || die "cargo not found.
  cutback is built from source and needs a Rust toolchain.
  Install one from https://rustup.rs then run this script again."

rust_version=$(rustc --version 2>/dev/null | awk '{print $2}')
[ -n "$rust_version" ] || die "could not determine the rustc version"

# Sort based comparison, so this keeps working past 1.9 to 1.10.
oldest=$(printf '%s\n%s\n' "$MIN_RUST" "$rust_version" | sort -V | head -n1)
if [ "$oldest" != "$MIN_RUST" ] && [ "$rust_version" != "$MIN_RUST" ]; then
    die "rustc $rust_version is too old, cutback needs $MIN_RUST or newer.
  Update with: rustup update stable"
fi
say "  rustc $rust_version ${dim}(need $MIN_RUST or newer)${reset}"

if command -v kdenlive >/dev/null 2>&1; then
    say "  kdenlive found"
else
    warn "kdenlive was not found on PATH. cutback will still install."
fi

# Run from the repository if the script sits in one, otherwise fetch it.
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
workdir=""
cleanup() { [ -n "$workdir" ] && rm -rf "$workdir"; }
trap cleanup EXIT INT TERM

if [ -f "$script_dir/Cargo.toml" ]; then
    src="$script_dir"
    say "  building from $src"
else
    command -v git >/dev/null 2>&1 || die "git is needed to fetch the source"
    workdir=$(mktemp -d)
    step "Fetching the source"
    git clone --depth 1 "$REPO_URL" "$workdir/cutback" >/dev/null 2>&1 \
        || die "could not clone $REPO_URL"
    src="$workdir/cutback"
fi

step "Building"
say "  ${dim}this takes a minute on a first build${reset}"
( cd "$src" && cargo build --release --locked 2>&1 | sed 's/^/  /' ) \
    || die "the build failed, see the output above"

binary="$src/target/release/cutback"
[ -x "$binary" ] || die "the build finished but $binary is missing"

step "Verifying"
"$binary" --version >/dev/null 2>&1 || die "the built binary does not run"
say "  $("$binary" --version)"

if [ -e "$bindir/cutback" ]; then
    existing=$("$bindir/cutback" --version 2>/dev/null || echo "unknown version")
    say ""
    say "  $bindir/cutback already exists ($existing)"
    confirm "  Replace it?" || die "nothing was changed"
fi

step "Installing to $prefix"
mkdir -p "$bindir" "$mandir" || die "could not create $bindir.
  Either pick a writable prefix, for example --prefix \$HOME/.local,
  or run this with sudo for a system wide install."

# Install to a temporary name and rename, so an interrupted copy cannot leave
# a half written binary that still looks executable.
install -m 755 "$binary" "$bindir/.cutback.new" || die "could not write to $bindir"
mv -f "$bindir/.cutback.new" "$bindir/cutback"
say "  $bindir/cutback"

if [ -f "$src/doc/cutback.1" ]; then
    install -m 644 "$src/doc/cutback.1" "$mandir/cutback.1" \
        && say "  $mandir/cutback.1"
fi

# Completions are a convenience, so a failure here should not fail the install.
install_completion() {
    dir="$1"; file="$2"; shell="$3"
    [ -n "$dir" ] || return 0
    mkdir -p "$dir" 2>/dev/null || return 0
    if "$binary" completions "$shell" >"$dir/$file" 2>/dev/null; then
        say "  $dir/$file"
    else
        rm -f "$dir/$file"
    fi
}

step "Installing shell completions"
install_completion "$destdir$prefix/share/bash-completion/completions" cutback bash
install_completion "$destdir$prefix/share/zsh/site-functions" _cutback zsh
install_completion "$destdir$prefix/share/fish/vendor_completions.d" cutback.fish fish

say ""
printf '%sInstalled.%s\n' "$green$bold" "$reset"

case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *)
        say ""
        warn "$prefix/bin is not on your PATH."
        case "${SHELL##*/}" in
            zsh)  rc="~/.zshrc" ;;
            bash) rc="~/.bashrc" ;;
            fish) rc="~/.config/fish/config.fish" ;;
            *)    rc="your shell's startup file" ;;
        esac
        say "Add this to $rc, then open a new terminal:"
        say ""
        if [ "${SHELL##*/}" = "fish" ]; then
            say "    fish_add_path $prefix/bin"
        else
            say "    export PATH=\"$prefix/bin:\$PATH\""
        fi
        ;;
esac

say ""
say "Get started:"
say "    cd /path/to/your/kdenlive/project"
say "    cutback watch"
say ""
say "Then edit and save in Kdenlive as usual, and run ${bold}cutback log${reset} to see the history."
say "Documentation: ${bold}man cutback${reset}"
