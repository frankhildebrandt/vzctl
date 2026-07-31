#!/usr/bin/env bash
# Fetch a pinned Caddy binary into daemon/Vendor/caddy/ (and optionally Application Support bin/).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${CADDY_VERSION:-2.9.1}"
DEST_DIR="$ROOT/daemon/Vendor/caddy"
mkdir -p "$DEST_DIR"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) CADDY_ARCH="arm64" ;;
  x86_64) CADDY_ARCH="amd64" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac
URL="https://github.com/caddyserver/caddy/releases/download/v${VERSION}/caddy_${VERSION}_darwin_${CADDY_ARCH}.tar.gz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/caddy.tgz"
tar -xzf "$TMP/caddy.tgz" -C "$TMP"
install -m 755 "$TMP/caddy" "$DEST_DIR/caddy"
echo "$VERSION" > "$DEST_DIR/VERSION"
echo "installed $DEST_DIR/caddy ($VERSION)"
# Optional runtime copy
RUNTIME_BIN="${HOME}/Library/Application Support/vzctl/bin"
mkdir -p "$RUNTIME_BIN"
install -m 755 "$DEST_DIR/caddy" "$RUNTIME_BIN/caddy"
echo "copied to $RUNTIME_BIN/caddy"
