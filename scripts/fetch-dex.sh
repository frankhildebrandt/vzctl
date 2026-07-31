#!/usr/bin/env bash
# Build a pinned Dex binary into daemon/Vendor/dex/ (+ Application Support bin/).
#
# Dex tags as v2.x but go.mod still says `module github.com/dexidp/dex` (no /v2),
# so `go install …@v2.x` fails. We clone the tag and `go build` locally instead.
# Prebuilt GitHub release assets are no longer published.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${DEX_VERSION:-2.45.1}"
DEST_DIR="$ROOT/daemon/Vendor/dex"
mkdir -p "$DEST_DIR"
if ! command -v go >/dev/null 2>&1; then
  echo "go is required to build dex" >&2
  exit 1
fi
if ! command -v git >/dev/null 2>&1; then
  echo "git is required to build dex" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "cloning dex v${VERSION}"
git clone --depth 1 --branch "v${VERSION}" https://github.com/dexidp/dex.git "$TMP/dex"
echo "building ./cmd/dex (CGO required for sqlite3 storage)"
(
  cd "$TMP/dex"
  # Dex sqlite3 storage needs modernc/mattn sqlite via CGO. Pure-Go builds
  # fail at runtime with "binary compiled without CGO support".
  CGO_ENABLED=1 go build -trimpath -ldflags "-s -w" -o "$DEST_DIR/dex" ./cmd/dex
)
if [ ! -x "$DEST_DIR/dex" ]; then
  echo "dex binary missing after build" >&2
  exit 1
fi
echo "$VERSION" > "$DEST_DIR/VERSION"
RUNTIME_BIN="${HOME}/Library/Application Support/vzctl/bin"
mkdir -p "$RUNTIME_BIN"
install -m 755 "$DEST_DIR/dex" "$RUNTIME_BIN/dex"
echo "installed $DEST_DIR/dex ($VERSION)"
echo "copied to $RUNTIME_BIN/dex"
"$DEST_DIR/dex" version 2>/dev/null || true
