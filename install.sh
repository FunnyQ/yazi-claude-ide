#!/usr/bin/env bash
# Downloads the yazi-claude-ide sidecar binary from the latest GitHub release
# and installs it. Override the destination with YCI_INSTALL_DIR.
#
#   curl -sSL https://raw.githubusercontent.com/FunnyQ/yazi-claude-ide/main/install.sh | bash

set -euo pipefail

REPO="FunnyQ/yazi-claude-ide"
INSTALL_DIR="${YCI_INSTALL_DIR:-$HOME/.local/bin}"

die() { printf 'install.sh: %s\n' "$1" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "${os}/${arch}" in
  Darwin/arm64)              target="aarch64-apple-darwin" ;;
  Linux/x86_64)              target="x86_64-unknown-linux-musl" ;;
  Linux/aarch64|Linux/arm64) target="aarch64-unknown-linux-musl" ;;
  *) die "no prebuilt binary for ${os}/${arch} — published targets are macOS arm64,
  Linux x86_64, and Linux arm64.
  Build it instead: cargo install --root \$HOME/.local --git https://github.com/${REPO}" ;;
esac

command -v curl >/dev/null || die "curl is required"
command -v tar  >/dev/null || die "tar is required"

tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
[ -n "$tag" ] || die "could not read the latest release tag from the GitHub API"

asset="yazi-claude-ide-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

printf 'Downloading %s %s\n' "$REPO" "$tag"
curl -fsSL "$url" -o "$tmp/$asset" || die "download failed: $url"
tar -xzf "$tmp/$asset" -C "$tmp" || die "could not extract $asset"
[ -f "$tmp/yazi-claude-ide" ] || die "archive did not contain yazi-claude-ide"

mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/yazi-claude-ide" "$INSTALL_DIR/yazi-claude-ide"

printf 'Installed to %s/yazi-claude-ide\n' "$INSTALL_DIR"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) printf '\n%s is not on your PATH. Add this to your shell profile:\n\n  export PATH="%s:$PATH"\n' \
       "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac

# A second copy earlier on PATH — a leftover `cargo install` in ~/.cargo/bin, say —
# wins silently: yazi keeps forking the old binary and nothing reports a version.
resolved=$(command -v yazi-claude-ide 2>/dev/null || true)
if [ -n "$resolved" ] && [ "$resolved" != "${INSTALL_DIR}/yazi-claude-ide" ]; then
  printf '\nWarning: %s comes first on your PATH, so yazi will run that one.\n  Delete it, or move %s ahead of it.\n' \
    "$resolved" "$INSTALL_DIR"
fi
