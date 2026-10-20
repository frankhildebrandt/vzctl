#!/usr/bin/env python3
"""Fail packaging on mismatched versions or non-relocatable qemu-img dependencies."""
import json
import pathlib
import plistlib
import re
import subprocess
import sys


def run(argv):
    return subprocess.run(argv, check=True, text=True, capture_output=True, timeout=10).stdout.strip()


def validate(root, tag):
    def cargo(path):
        text = (root / path).read_text()
        match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
        if not match:
            raise ValueError(f"missing package version in {path}")
        return match.group(1)

    versions = {
        name: cargo(path)
        for name, path in {
            "cli": "crates/vzctl/Cargo.toml",
            "mcp": "crates/vzctl-mcp/Cargo.toml",
            "oidc_simple": "crates/vzctl-oidc-simple/Cargo.toml",
            "tauri_rust": "apps/vzctl-ui/src-tauri/Cargo.toml",
        }.items()
    }
    for name, path in {
        "tauri": "apps/vzctl-ui/src-tauri/tauri.conf.json",
        "ui": "apps/vzctl-ui/package.json",
    }.items():
        versions[name] = json.loads((root / path).read_text())["version"]
    swift = (root / "daemon/Sources/VzDaemonKit/Types.swift").read_text()
    versions["swift"] = re.search(r'static let version = "([^"]+)"', swift).group(1)
    app = root / "apps/vzctl-ui/src-tauri/target/release/bundle/macos/vzctl.app/Contents/Info.plist"
    versions["app_bundle"] = plistlib.loads(app.read_bytes())["CFBundleShortVersionString"]
    version = versions["cli"]
    if any(value != version for value in versions.values()):
        raise ValueError(f"version mismatch; rebuild from consistent sources: {versions}")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?(?:\+[A-Za-z0-9.-]+)?", version):
        raise ValueError(f"unsupported source version {version!r}")
    if tag != "dev" and tag.removeprefix("v") != version:
        raise ValueError(f"release tag {tag!r} does not match source version {version!r}; choose the release version explicitly")
    binaries = {
        "vzctl": ("target/release/vzctl", ["version", "--format", "human"]),
        "vzctl-mcp": ("target/release/vzctl-mcp", ["--version"]),
        "vzctl-oidc-simple": ("target/release/vzctl-oidc-simple", ["--version"]),
    }
    for name in ["vz-net", "vz-edge", "vz-supervisor", "vz-helper", "vz-dns-bind"]:
        binaries[name] = (f"daemon/.build/release/{name}", ["version"])
    for name, (path, args) in binaries.items():
        output = run([str(root / path), *args]).splitlines()[0].split()
        if output[:2] != [name, version]:
            raise ValueError(f"stale binary version for {name}; rebuild release binaries")

    qemu = root / "daemon/Vendor/qemu-img"
    for binary in [qemu / "qemu-img", *sorted((qemu / "lib").glob("*.dylib"))]:
        for line in run(["otool", "-L", str(binary)]).splitlines()[1:]:
            # Universal Mach-O emits another unindented header per architecture.
            # Check dependencies from every slice, not just the host architecture.
            if re.fullmatch(re.escape(str(binary)) + r" \(architecture [^)]+\):", line):
                continue
            dependency = line.strip().split(" (", 1)[0]
            if dependency.startswith(("/usr/lib/", "/System/")):
                continue
            if dependency.startswith("@rpath/"):
                resolved = qemu / "lib" / dependency.removeprefix("@rpath/")
            elif dependency.startswith("@loader_path/"):
                resolved = binary.parent / dependency.removeprefix("@loader_path/")
            elif dependency.startswith("@executable_path/"):
                resolved = qemu / dependency.removeprefix("@executable_path/")
            else:
                raise ValueError(f"qemu-img has non-relocatable dependency {dependency!r}")
            if not resolved.is_file() or not resolved.resolve().is_relative_to(qemu.resolve()):
                raise ValueError(f"qemu-img bundled dependency is missing or escapes bundle: {dependency!r}")
    try:
        commit = run(["git", "-C", str(root), "rev-parse", "HEAD"])
        dirty = bool(run(["git", "-C", str(root), "status", "--porcelain", "--untracked-files=no"]))
    except subprocess.CalledProcessError:
        commit, dirty = "unknown", True
    return {
        "version": version, "pkg_version": version.split("-", 1)[0].split("+", 1)[0],
        "tag": tag, "git_commit": commit, "dirty": dirty, "component_versions": versions,
        "vendor_versions": {name: (root / f"daemon/Vendor/{name}/VERSION").read_text().strip()
                            for name in ["caddy", "dex", "qemu-img"]},
    }


if __name__ == "__main__":
    try:
        print(json.dumps(validate(pathlib.Path(sys.argv[1]), sys.argv[2]), indent=2))
    except (ValueError, OSError, KeyError, AttributeError, IndexError, subprocess.SubprocessError) as error:
        print(f"package-macos: {error}", file=sys.stderr)
        sys.exit(1)
