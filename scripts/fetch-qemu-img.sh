#!/usr/bin/env bash
# Bundle a relocatable qemu-img (+ dylibs) into daemon/Vendor/qemu-img/.
#
# Source order:
#   1. QEMU_IMG_SRC (existing qemu-img binary)
#   2. qemu-img on PATH
#   3. Multipass helper at /Library/Application Support/com.canonical.multipass
#   4. pinned QEMU source build (needs meson/ninja/pkg-config/glib)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST_DIR="$ROOT/daemon/Vendor/qemu-img"
QEMU_VERSION="${QEMU_VERSION:-11.1.0}"
QEMU_SHA256="${QEMU_SHA256:-6ee1d1a61f68212476b27108c26da5f449dc09b626d42f8279ba0dc2e08fa858}"
MULTIPASS_BIN="/Library/Application Support/com.canonical.multipass/bin/qemu-img"
MULTIPASS_LIB="/Library/Application Support/com.canonical.multipass/lib"
RUNTIME_LIBEXEC="${HOME}/Library/Application Support/vzctl/libexec/qemu-img"

ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) OTOOL_ARCH=arm64 ;;
  x86_64) OTOOL_ARCH=x86_64 ;;
  *)
    echo "unsupported arch: $ARCH" >&2
    exit 1
    ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

is_system_lib() {
  case "$1" in
    /usr/lib/*|/System/*) return 0 ;;
    *) return 1 ;;
  esac
}

list_load_dylibs() {
  otool -arch "$OTOOL_ARCH" -L "$1" | awk '
    NR == 1 { next }
    {
      sub(/^[ \t]+/, "")
      split($0, parts, " ")
      if (parts[1] != "") print parts[1]
    }
  '
}

resolve_dylib() {
  local spec=$1
  local base dir candidate
  case "$spec" in
    @rpath/*|@executable_path/*|@loader_path/*)
      base="${spec##*/}"
      for dir in "${SEARCH_DIRS[@]}"; do
        candidate="$dir/$base"
        if [ -f "$candidate" ]; then
          printf '%s\n' "$candidate"
          return 0
        fi
      done
      ;;
    /*)
      if [ -f "$spec" ]; then
        printf '%s\n' "$spec"
        return 0
      fi
      base="$(basename "$spec")"
      for dir in "${SEARCH_DIRS[@]}"; do
        candidate="$dir/$base"
        if [ -f "$candidate" ]; then
          printf '%s\n' "$candidate"
          return 0
        fi
      done
      ;;
  esac
  return 1
}

ensure_rpath() {
  local macho=$1
  local rpath=$2
  if otool -arch "$OTOOL_ARCH" -l "$macho" | grep -q "path $rpath "; then
    return 0
  fi
  install_name_tool -add_rpath "$rpath" "$macho"
}

bundle_qemu_img() {
  local source=$1
  local version_label=$2
  local libdir="$DEST_DIR/lib"
  rm -rf "$DEST_DIR"
  mkdir -p "$libdir"
  install -m 755 "$source" "$DEST_DIR/qemu-img"
  codesign --remove-signature "$DEST_DIR/qemu-img" >/dev/null 2>&1 || true

  SEARCH_DIRS=("$libdir")
  SEARCH_DIRS+=("$(dirname "$source")")
  SEARCH_DIRS+=("$(cd "$(dirname "$source")/.." && pwd)/lib")
  SEARCH_DIRS+=("$MULTIPASS_LIB")
  if command -v brew >/dev/null 2>&1; then
    SEARCH_DIRS+=("$(brew --prefix)/lib")
    local opt
    for opt in glib gettext pcre2 zstd pixman; do
      if [ -d "$(brew --prefix "$opt" 2>/dev/null)/lib" ]; then
        SEARCH_DIRS+=("$(brew --prefix "$opt")/lib")
      fi
    done
  fi

  local pending=("$DEST_DIR/qemu-img")
  local seen="|"
  local macho spec resolved base destlib
  while [ "${#pending[@]}" -gt 0 ]; do
    macho="${pending[0]}"
    pending=("${pending[@]:1}")
    while IFS= read -r spec; do
      [ -n "$spec" ] || continue
      if is_system_lib "$spec"; then
        continue
      fi
      if [ "$spec" = "$macho" ]; then
        continue
      fi
      resolved="$(resolve_dylib "$spec" || true)"
      if [ -z "$resolved" ]; then
        echo "cannot resolve dylib $spec (needed by $macho)" >&2
        exit 1
      fi
      base="$(basename "$resolved")"
      destlib="$libdir/$base"
      if [ -z "${seen##*|"$base"|*}" ]; then
        if [ "$spec" != "@rpath/$base" ]; then
          install_name_tool -change "$spec" "@rpath/$base" "$macho"
        fi
        continue
      fi
      seen="${seen}${base}|"
      install -m 644 "$resolved" "$destlib"
      codesign --remove-signature "$destlib" >/dev/null 2>&1 || true
      install_name_tool -id "@rpath/$base" "$destlib"
      pending+=("$destlib")
      if [ "$spec" != "@rpath/$base" ]; then
        install_name_tool -change "$spec" "@rpath/$base" "$macho"
      fi
    done < <(list_load_dylibs "$macho")
  done

  ensure_rpath "$DEST_DIR/qemu-img" "@executable_path/lib"
  install_name_tool -delete_rpath "@executable_path/../lib" "$DEST_DIR/qemu-img" >/dev/null 2>&1 || true
  for destlib in "$libdir"/*; do
    [ -f "$destlib" ] || continue
    ensure_rpath "$destlib" "@loader_path"
    ensure_rpath "$destlib" "@executable_path/lib"
    codesign --force --sign - --timestamp=none "$destlib" >/dev/null
  done
  codesign --force --sign - --timestamp=none "$DEST_DIR/qemu-img" >/dev/null

  printf '%s\n' "$version_label" >"$DEST_DIR/VERSION"
  cat >"$DEST_DIR/LICENSE" <<'EOF'
qemu-img is part of QEMU and licensed under GPL-2.0-only.
See https://www.qemu.org/ and https://gitlab.com/qemu-project/qemu
EOF
}

build_from_source() {
  for tool in meson ninja pkg-config; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "source build needs $tool (brew install meson ninja pkg-config glib)" >&2
      return 1
    fi
  done
  if ! pkg-config --exists glib-2.0; then
    echo "source build needs glib-2.0 (brew install glib)" >&2
    return 1
  fi
  local tarball="$TMP/qemu.tar.xz"
  local url="https://download.qemu.org/qemu-${QEMU_VERSION}.tar.xz"
  echo "downloading $url"
  curl -fsSL "$url" -o "$tarball"
  local actual
  actual="$(shasum -a 256 "$tarball" | awk '{print $1}')"
  if [ "$actual" != "$QEMU_SHA256" ]; then
    echo "qemu tarball sha256 mismatch: $actual (expected $QEMU_SHA256)" >&2
    return 1
  fi
  tar -xJf "$tarball" -C "$TMP"
  local src="$TMP/qemu-${QEMU_VERSION}"
  echo "configuring qemu-img ${QEMU_VERSION}"
  (
    cd "$src"
    ./configure \
      --target-list= \
      --disable-system \
      --disable-user \
      --disable-docs \
      --disable-guest-agent \
      --disable-slirp \
      --disable-capstone \
      --disable-fdt \
      --enable-tools
    ninja -C build qemu-img
  )
  bundle_qemu_img "$src/build/qemu-img" "$QEMU_VERSION"
}

find_source_binary() {
  if [ -n "${QEMU_IMG_SRC:-}" ]; then
    if [ ! -x "$QEMU_IMG_SRC" ]; then
      echo "QEMU_IMG_SRC is not executable: $QEMU_IMG_SRC" >&2
      exit 1
    fi
    printf '%s\n' "$QEMU_IMG_SRC"
    return 0
  fi
  if command -v qemu-img >/dev/null 2>&1; then
    command -v qemu-img
    return 0
  fi
  if [ -x "$MULTIPASS_BIN" ]; then
    printf '%s\n' "$MULTIPASS_BIN"
    return 0
  fi
  return 1
}

SOURCE="$(find_source_binary || true)"
if [ -n "$SOURCE" ]; then
  VERSION_LABEL="$("$SOURCE" --version 2>/dev/null | awk 'NR==1 {print $3; exit}')"
  VERSION_LABEL="${VERSION_LABEL:-bundled}"
  echo "bundling $SOURCE ($VERSION_LABEL)"
  bundle_qemu_img "$SOURCE" "$VERSION_LABEL"
else
  echo "no local qemu-img; building ${QEMU_VERSION} from source"
  build_from_source
fi

if [ ! -x "$DEST_DIR/qemu-img" ]; then
  echo "qemu-img missing after vendor" >&2
  exit 1
fi
"$DEST_DIR/qemu-img" --version

mkdir -p "$RUNTIME_LIBEXEC"
ditto "$DEST_DIR" "$RUNTIME_LIBEXEC"
echo "installed $DEST_DIR/qemu-img"
echo "copied to $RUNTIME_LIBEXEC/qemu-img"
