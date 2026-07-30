#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOLD="${1:-120}"
CMD="${2:-nets}"
BIN=".build/debug/G0Spike"

swift build
codesign --force --sign - --entitlements "$ROOT/G0Spike.entitlements" "$BIN"
exec "$BIN" "$CMD" "$HOLD"
