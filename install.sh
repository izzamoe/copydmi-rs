#!/usr/bin/env bash
# install.sh — one-liner installer for copydmi (Linux/macOS)
#
# USAGE:
#   curl -fsSL https://raw.githubusercontent.com/izzamoe/copydmi-rs/master/install.sh | bash
#
# Downloads the latest release binary from GitHub Releases, verifies its
# SHA-256 checksum against SHA256SUMS.txt, and installs it to a directory
# on your PATH (defaults to ~/.local/bin, falls back to /usr/local/bin with
# sudo if writable, or lets you override with INSTALL_DIR=/some/path).

set -euo pipefail

REPO="izzamoe/copydmi-rs"
BIN_NAME="copydmi"

echo "== Installing ${BIN_NAME} =="

# Detect OS/arch — this project currently ships linux-x86_64 only.
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux) ;;
    Darwin)
        echo "error: macOS builds are not published yet. Build from source instead:" >&2
        echo "  git clone https://github.com/${REPO}.git && cd copydmi-rs && cargo build --release" >&2
        exit 1
        ;;
    *)
        echo "error: unsupported OS '$OS'. This installer supports Linux only (use install.ps1 on Windows)." >&2
        exit 1
        ;;
esac

case "$ARCH" in
    x86_64|amd64) ASSET="copydmi-linux-x86_64" ;;
    *)
        echo "error: unsupported architecture '$ARCH'. Only x86_64 builds are published." >&2
        exit 1
        ;;
esac

# Resolve latest release tag via GitHub API (no auth needed for public repo).
echo "-- resolving latest release --"
LATEST_JSON="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")"
TAG="$(echo "$LATEST_JSON" | grep -o '"tag_name": *"[^"]*"' | head -1 | sed -E 's/.*"([^"]+)"$/\1/')"

if [[ -z "$TAG" ]]; then
    echo "error: could not determine latest release tag. Check https://github.com/${REPO}/releases" >&2
    exit 1
fi

echo "  latest release: $TAG"

BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "-- downloading ${ASSET} --"
curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMPDIR}/${ASSET}"
curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMPDIR}/SHA256SUMS.txt" || {
    echo "warning: could not fetch SHA256SUMS.txt, skipping checksum verification" >&2
}

if [[ -f "${TMPDIR}/SHA256SUMS.txt" ]]; then
    echo "-- verifying checksum --"
    EXPECTED="$(grep " ${ASSET}\$" "${TMPDIR}/SHA256SUMS.txt" | awk '{print $1}')"
    ACTUAL="$(sha256sum "${TMPDIR}/${ASSET}" | awk '{print $1}')"
    if [[ -z "$EXPECTED" ]]; then
        echo "warning: no checksum entry found for ${ASSET}, skipping verification" >&2
    elif [[ "$EXPECTED" != "$ACTUAL" ]]; then
        echo "error: checksum mismatch! expected $EXPECTED, got $ACTUAL" >&2
        echo "       refusing to install a corrupted/tampered binary." >&2
        exit 1
    else
        echo "  checksum OK ($ACTUAL)"
    fi
fi

# Pick an install directory: respect $INSTALL_DIR, else prefer ~/.local/bin
# (no sudo needed, standard user-writable PATH dir on most distros), else
# fall back to /usr/local/bin (may need sudo).
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$INSTALL_DIR"

chmod +x "${TMPDIR}/${ASSET}"
cp "${TMPDIR}/${ASSET}" "${INSTALL_DIR}/${BIN_NAME}"

echo "-- installed --"
echo "  ${INSTALL_DIR}/${BIN_NAME}"

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        echo "  '$INSTALL_DIR' is already on your PATH."
        ;;
    *)
        echo "  NOTE: '$INSTALL_DIR' is not on your PATH. Add this to your shell profile:"
        echo "    export PATH=\"\$PATH:$INSTALL_DIR\""
        ;;
esac

echo ""
echo "Run '${BIN_NAME} --help' to get started."
