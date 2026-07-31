#!/usr/bin/env bash
# Fetch a pinned Dex binary into daemon/Vendor/dex/.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${DEX_VERSION:-2.42.0}"
DEST_DIR="$ROOT/daemon/Vendor/dex"
mkdir -p "$DEST_DIR"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) DEX_ARCH="arm64" ;;
  x86_64) DEX_ARCH="amd64" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac
URL="https://github.com/dexidp/dex/releases/download/v${VERSION}/dex_${VERSION}_darwin_${DEX_ARCH}.tar.gz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
if ! curl -fsSL "$URL" -o "$TMP/dex.tgz"; then
  # Fallback: go install into Vendor (requires Go).
  echo "release tarball unavailable; building with go install" >&2
  GOBIN="$DEST_DIR" go install "github.com/dexidp/dex/cmd/dex@v${VERSION}"
  echo "$VERSION" > "$DEST_DIR/VERSION"
  RUNTIME_BIN="${HOME}/Library/Application Support/vzctl/bin"
  mkdir -p "$RUNTIME_BIN"
  install -m 755 "$DEST_DIR/dex" "$RUNTIME_BIN/dex"
  echo "installed $DEST_DIR/dex ($VERSION)"
  exit 0
fi
tar -xzf "$TMP/dex.tgz" -C "$TMP"
BIN="$(find "$TMP" -type f -name dex | head -1)"
install -m 755 "$BIN" "$DEST_DIR/dex"
echo "$VERSION" > "$DEST_DIR/VERSION"
RUNTIME_BIN="${HOME}/Library/Application Support/vzctl/bin"
mkdir -p "$RUNTIME_BIN"
install -m 755 "$DEST_DIR/dex" "$RUNTIME_BIN/dex"
echo "installed $DEST_DIR/dex ($VERSION)"
