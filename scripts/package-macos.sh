#!/bin/sh
# Build macOS release artefacts: tar.gz (CLI), .pkg (installer), .dmg (app disk image).
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tag=${RELEASE_TAG:-}
if [ -z "$tag" ]; then
  if command -v git >/dev/null 2>&1 && git -C "$root" describe --tags --exact-match >/dev/null 2>&1; then
    tag=$(git -C "$root" describe --tags --exact-match)
  else
    tag=dev
  fi
fi
version=${tag#v}
# pkgbuild wants a dotted numeric version; fall back for local/dev tags.
case "$version" in
  [0-9]*) ;;
  *) version=0.0.0 ;;
esac

arch=$(uname -m)
case "$arch" in
  arm64|aarch64) arch_label=arm64 ;;
  x86_64) arch_label=amd64 ;;
  *) arch_label=$arch ;;
esac

dist="$root/dist"
name="vzctl-${tag}-darwin-${arch_label}"
tar_dir="$dist/${name}"
pkg_root="$dist/pkgroot-${name}"
dmg_stage="$dist/dmg-${name}"
scripts_dir="$dist/pkgscripts-${name}"

cli="$root/target/release/vzctl"
helper="$root/daemon/.build/release/vz-helper"
supervisor="$root/daemon/.build/release/vz-supervisor"
net="$root/daemon/.build/release/vz-net"
edge="$root/daemon/.build/release/vz-edge"
dns_bind="$root/daemon/.build/release/vz-dns-bind"
oidc_simple="$root/target/release/vzctl-oidc-simple"
tauri_app="$root/apps/vzctl-ui/src-tauri/target/release/bundle/macos/vzctl.app"
postinstall_src="$root/packaging/macos/postinstall"

die() {
  echo "package-macos: $*" >&2
  exit 1
}

need_file() {
  [ -f "$1" ] || die "missing $1 (run make release first)"
}

need_dir() {
  [ -d "$1" ] || die "missing $1 (run make ui-build first)"
}

need_file "$cli"
need_file "$helper"
need_file "$supervisor"
need_file "$net"
need_file "$edge"
need_file "$dns_bind"
need_dir "$tauri_app"
[ -f "$postinstall_src" ] || die "missing $postinstall_src"

rm -rf "$tar_dir" "$pkg_root" "$dmg_stage" "$scripts_dir"
mkdir -p "$dist" "$tar_dir" "$pkg_root/usr/local/bin" "$pkg_root/Applications" \
  "$dmg_stage" "$scripts_dir"

# --- tar.gz (CLI + daemons) -------------------------------------------------
cp "$cli" "$helper" "$supervisor" "$net" "$edge" "$dns_bind" "$tar_dir/"
if [ -x "$oidc_simple" ]; then
  cp "$oidc_simple" "$tar_dir/"
fi
cp "$root/README.md" "$tar_dir/"
tar -C "$dist" -czf "$dist/${name}.tar.gz" "$name"
rm -rf "$tar_dir"
shasum -a 256 "$dist/${name}.tar.gz" >"$dist/${name}.tar.gz.sha256"

# --- .pkg (app + binaries + LaunchAgents via postinstall) -------------------
install -m 0755 "$cli" "$pkg_root/usr/local/bin/vzctl"
install -m 0755 "$net" "$pkg_root/usr/local/bin/vz-net"
install -m 0755 "$edge" "$pkg_root/usr/local/bin/vz-edge"
install -m 0755 "$supervisor" "$pkg_root/usr/local/bin/vz-supervisor"
install -m 0755 "$helper" "$pkg_root/usr/local/bin/vz-helper"
install -m 0755 "$dns_bind" "$pkg_root/usr/local/bin/vz-dns-bind"
if [ -x "$oidc_simple" ]; then
  install -m 0755 "$oidc_simple" "$pkg_root/usr/local/bin/vzctl-oidc-simple"
fi
ditto "$tauri_app" "$pkg_root/Applications/vzctl.app"

install -m 0755 "$postinstall_src" "$scripts_dir/postinstall"

pkgbuild \
  --root "$pkg_root" \
  --scripts "$scripts_dir" \
  --identifier dev.vzctl.installer \
  --version "$version" \
  --install-location / \
  "$dist/${name}.pkg"

shasum -a 256 "$dist/${name}.pkg" >"$dist/${name}.pkg.sha256"
rm -rf "$pkg_root" "$scripts_dir"

# --- .dmg (app + Applications link + pkg) -----------------------------------
ditto "$tauri_app" "$dmg_stage/vzctl.app"
ln -s /Applications "$dmg_stage/Applications"
cp "$dist/${name}.pkg" "$dmg_stage/Install vzctl.pkg"
cat >"$dmg_stage/README.txt" <<EOF
vzctl ${tag}

Recommended: double-click "Install vzctl.pkg"
  → installs /Applications/vzctl.app, CLI under /usr/local/bin,
    and LaunchAgents for the logged-in user.

Alternatively: drag vzctl.app onto Applications (GUI only).
EOF

tmp_dmg="$dist/${name}.tmp.dmg"
rm -f "$tmp_dmg" "$dist/${name}.dmg"
hdiutil create \
  -volname "vzctl ${tag}" \
  -srcfolder "$dmg_stage" \
  -ov \
  -format UDRW \
  "$tmp_dmg" >/dev/null
hdiutil convert "$tmp_dmg" -format UDZO -imagekey zlib-level=9 -o "$dist/${name}.dmg" >/dev/null
rm -f "$tmp_dmg"
rm -rf "$dmg_stage"
shasum -a 256 "$dist/${name}.dmg" >"$dist/${name}.dmg.sha256"

echo "package-macos: wrote"
ls -1 "$dist/${name}.tar.gz" "$dist/${name}.pkg" "$dist/${name}.dmg" \
  "$dist/${name}.tar.gz.sha256" "$dist/${name}.pkg.sha256" "$dist/${name}.dmg.sha256"
