#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ASSETS="$ROOT/assets"
mkdir -p "$ASSETS"
cd "$ASSETS"
IMG=ubuntu-24.04-server-cloudimg-arm64.img
URL=https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.img
[[ -f $IMG ]] || curl -L -o "$IMG" "$URL"
[[ -f base.raw ]] || { qemu-img convert -f qcow2 -O raw "$IMG" base.raw; qemu-img resize -f raw base.raw 8G; }
rm -f frontend.raw backend.raw
cp -c base.raw frontend.raw
cp -c base.raw backend.raw
command -v xorriso >/dev/null || { echo "need xorriso (brew install xorriso)"; exit 1; }
xorriso -as mkisofs -R -V cidata -o cidata-fe.iso cidata-fe
xorriso -as mkisofs -R -V cidata -o cidata-be.iso cidata-be
if [[ -d cidata-router ]]; then
  rm -f router.raw
  cp -c base.raw router.raw
  xorriso -as mkisofs -R -V cidata -o cidata-router.iso cidata-router
fi
echo "assets ready in $ASSETS"
