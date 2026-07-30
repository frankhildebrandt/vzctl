#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
daemon_dir=$(dirname "$script_dir")
helper_path=${1:-"$daemon_dir/.build/debug/vz-helper"}

codesign --force --sign - \
  --entitlements "$daemon_dir/VzHelper.entitlements" \
  "$helper_path"
codesign --verify --strict "$helper_path"
codesign -d --entitlements - "$helper_path"
