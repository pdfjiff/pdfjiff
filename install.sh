#!/bin/sh
# PDFJiff installer for macOS and Linux.
#
#   curl -fsSL https://github.com/pdfjiff/pdfjiff/releases/latest/download/install.sh | sh
#
# Options (pass after `sh -s --`, e.g. `| sh -s -- --version 0.1.0`):
#   --version VERSION   Install a specific release (default: latest)
#   --to DIR            Install directory (default: ~/.local/bin)
#   --uninstall         Remove pdfjiff from the install directory
#   -h, --help          Show this help
#
# Environment variables with the same effect: PDFJIFF_VERSION, PDFJIFF_INSTALL_DIR.
# PDFJIFF_DOWNLOAD_BASE overrides the download location (mirrors, air-gapped
# installs, tests); it must contain the release archives and SHA256SUMS.
#
# The script downloads one archive for your OS and CPU, verifies its SHA-256
# checksum, and copies a single binary into place. It never uses sudo and
# never edits your shell configuration.

set -eu

REPO="${PDFJIFF_REPO:-pdfjiff/pdfjiff}"
VERSION="${PDFJIFF_VERSION:-latest}"
INSTALL_DIR="${PDFJIFF_INSTALL_DIR:-${HOME}/.local/bin}"
BASE_URL="${PDFJIFF_DOWNLOAD_BASE:-}"
ACTION="install"

say() { printf 'pdfjiff-install: %s\n' "$*"; }
err() {
    printf 'pdfjiff-install: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'USAGE'
Install the pdfjiff CLI for macOS or Linux.

Usage: install.sh [--version VERSION] [--to DIR] [--uninstall]

  --version VERSION   Install a specific release, e.g. 0.1.0 (default: latest)
  --to DIR            Install directory (default: ~/.local/bin)
  --uninstall         Remove pdfjiff from the install directory
  -h, --help          Show this help

Environment: PDFJIFF_VERSION, PDFJIFF_INSTALL_DIR, PDFJIFF_DOWNLOAD_BASE
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || err "--version needs a value, for example --version 0.1.0"
            VERSION="$2"
            shift 2
            ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --to | --bin-dir)
            [ $# -ge 2 ] || err "--to needs a directory"
            INSTALL_DIR="$2"
            shift 2
            ;;
        --to=* | --bin-dir=*) INSTALL_DIR="${1#*=}"; shift ;;
        --uninstall) ACTION="uninstall"; shift ;;
        -h | --help) usage; exit 0 ;;
        *) err "unknown option: $1 (use --help)" ;;
    esac
done

BIN="${INSTALL_DIR}/pdfjiff"

if [ "$ACTION" = "uninstall" ]; then
    if [ -e "$BIN" ]; then
        rm -f "$BIN"
        say "removed ${BIN}"
    else
        say "nothing to remove at ${BIN}"
    fi
    exit 0
fi

# --- Detect the release target -------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux) os_part="unknown-linux-musl" ;; # static binary: works on every distro
    MINGW* | MSYS* | CYGWIN* | Windows_NT)
        err "on Windows, run in PowerShell: irm https://github.com/${REPO}/releases/latest/download/install.ps1 | iex"
        ;;
    *) err "unsupported operating system: ${os}. Build from source: cargo install pdfjiff-cli --locked" ;;
esac
case "$arch" in
    x86_64 | amd64) arch_part="x86_64" ;;
    arm64 | aarch64) arch_part="aarch64" ;;
    *) err "unsupported CPU architecture: ${arch}. Build from source: cargo install pdfjiff-cli --locked" ;;
esac
# An x86_64 shell under Rosetta on Apple silicon should still get the native build.
if [ "$os" = "Darwin" ] && [ "$arch_part" = "x86_64" ] &&
    [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
    arch_part="aarch64"
fi
TARGET="${arch_part}-${os_part}"
ARCHIVE="pdfjiff-${TARGET}.tar.gz"

if [ -z "$BASE_URL" ]; then
    case "$VERSION" in
        latest) BASE_URL="https://github.com/${REPO}/releases/latest/download" ;;
        v*) BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}" ;;
        *) BASE_URL="https://github.com/${REPO}/releases/download/v${VERSION}" ;;
    esac
fi

# --- Tools --------------------------------------------------------------------
download() { # url destination
    case "$1" in
        https://*) proto_args="--proto =https --tlsv1.2" ;;
        *) proto_args="" ;; # PDFJIFF_DOWNLOAD_BASE may point at a local mirror
    esac
    if command -v curl >/dev/null 2>&1; then
        # shellcheck disable=SC2086 # intentional word splitting of proto_args
        curl -fsSL $proto_args --retry 3 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        err "curl or wget is required"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        err "sha256sum, shasum or openssl is required to verify the download"
    fi
}

command -v tar >/dev/null 2>&1 || err "tar is required"

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t pdfjiff)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM

# --- Download and verify --------------------------------------------------------
say "downloading ${ARCHIVE} (${VERSION})"
download "${BASE_URL}/${ARCHIVE}" "${tmp}/${ARCHIVE}" ||
    err "download failed: ${BASE_URL}/${ARCHIVE}
  Check the version exists: https://github.com/${REPO}/releases"
download "${BASE_URL}/SHA256SUMS" "${tmp}/SHA256SUMS" ||
    err "could not download SHA256SUMS from ${BASE_URL}"

expected="$(awk -v f="$ARCHIVE" '{name=$2; sub(/^\*/, "", name); if (name == f) print $1}' "${tmp}/SHA256SUMS")"
[ -n "$expected" ] || err "SHA256SUMS has no entry for ${ARCHIVE}"
actual="$(sha256_of "${tmp}/${ARCHIVE}")"
[ "$expected" = "$actual" ] || err "checksum mismatch for ${ARCHIVE}
  expected ${expected}
  actual   ${actual}
The download may be corrupted or tampered with. Nothing was installed."
say "checksum verified"

tar -xzf "${tmp}/${ARCHIVE}" -C "$tmp"
[ -f "${tmp}/pdfjiff-${TARGET}/pdfjiff" ] || err "archive does not contain pdfjiff-${TARGET}/pdfjiff"

# --- Install ------------------------------------------------------------------
mkdir -p "$INSTALL_DIR" 2>/dev/null ||
    err "cannot create ${INSTALL_DIR}. Choose another directory with --to DIR"
[ -w "$INSTALL_DIR" ] ||
    err "${INSTALL_DIR} is not writable. Choose a directory you own with --to DIR,
  or install system-wide: curl -fsSL ... | sudo sh -s -- --to /usr/local/bin"
# Copy next to the destination, then rename: a running pdfjiff is never half-replaced.
cp "${tmp}/pdfjiff-${TARGET}/pdfjiff" "${INSTALL_DIR}/.pdfjiff.new.$$"
chmod 755 "${INSTALL_DIR}/.pdfjiff.new.$$"
mv -f "${INSTALL_DIR}/.pdfjiff.new.$$" "$BIN"

installed="$("$BIN" --version 2>/dev/null)" || err "installed ${BIN}, but it does not run on this system"
say "installed ${installed} to ${BIN}"

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        shell_name="$(basename "${SHELL:-sh}")"
        case "$shell_name" in
            zsh) rc="${ZDOTDIR:-$HOME}/.zshrc"; line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
            bash) rc="$HOME/.bashrc"; line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
            fish) rc=""; line="fish_add_path ${INSTALL_DIR}" ;;
            *) rc="$HOME/.profile"; line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
        esac
        echo
        say "${INSTALL_DIR} is not on your PATH. To fix that, run:"
        if [ -n "$rc" ]; then
            printf '\n    echo '\''%s'\'' >> %s && . %s\n\n' "$line" "$rc" "$rc"
        else
            printf '\n    %s\n\n' "$line"
        fi
        ;;
esac
say "next: pdfjiff --help    (tab completion: pdfjiff completions --help)"
