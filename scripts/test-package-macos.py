#!/usr/bin/env python3
"""Package/installer contract tests using fake binaries, services and package tools."""
import hashlib
import json
import os
import pathlib
import plistlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUST_BINARIES = ["vzctl", "vzctl-mcp", "vzctl-oidc-simple"]
SWIFT_BINARIES = ["vz-net", "vz-edge", "vz-supervisor", "vz-helper", "vz-dns-bind"]

FAKE_TOOL = r'''
import json, os, pathlib, shutil, socket, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ["PACKAGE_TEST_ROOT"])
log = root / "calls"
def record():
    with log.open("a") as stream:
        stream.write(json.dumps([tool, *args]) + "\n")
if tool == "stat":
    print(os.environ.get("CONSOLE_USER", "installer-test"))
elif tool == "dscl":
    print("NFSHomeDirectory: " + str(root / "home with spaces"))
elif tool == "id":
    print("501")
elif tool == "sudo":
    os.execvp(args[3],args[3:])
elif tool in ("chown", "sleep"):
    pass
elif tool == "pgrep":
    sys.exit(int(os.environ.get("PGREP_RESULT", "1")))
elif tool in ("helper-stop", "stop-vz-helpers.sh"):
    record()
    sys.exit(int(os.environ.get("HELPER_RESULT", "0")))
elif tool == "launchctl":
    state = root / "loaded.json"
    loaded = json.loads(state.read_text()) if state.exists() else []
    label = args[-1].split("/")[-1].removesuffix(".plist")
    if args[0] == "print":
        sys.exit(0 if label in loaded else 1)
    record()
    if args[0] == "bootout":
        if os.environ.get("STOP_FAILURE") != label:
            loaded = [item for item in loaded if item != label]
    elif args[0] == "bootstrap":
        if os.environ.get("START_FAILURE") == label:
            sys.exit(7)
        loaded.append(label)
        directory = root / "home with spaces/Library/Application Support/vzctl"
        directory.mkdir(parents=True, exist_ok=True)
        names = {"com.vzctl.net":["net.sock"],"com.vzctl.edge":["edge.sock"],
                 "com.vzctl.supervisor":["vz.sock","api.sock"]}[label]
        for name in names:
            stream=socket.socket(socket.AF_UNIX)
            stream.bind(str(directory / name))
            stream.close()
    state.write_text(json.dumps(loaded))
elif tool == "ditto":
    source, target = map(pathlib.Path, args)
    if source.is_dir(): shutil.copytree(source, target, dirs_exist_ok=True)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source,target)
elif tool == "otool":
    print(args[-1]+":")
    if os.environ.get("UNIVERSAL_QEMU"):
        print("\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)")
        print(args[-1]+" (architecture arm64):")
    if os.environ.get("BAD_QEMU"):
        print("\t/opt/homebrew/lib/unbundled.dylib (compatibility version 1.0.0)")
elif tool == "pkgbuild":
    source=pathlib.Path(args[args.index("--root")+1])
    scripts=pathlib.Path(args[args.index("--scripts")+1])
    shutil.copytree(source,root/"captured-payload")
    shutil.copytree(scripts,root/"captured-scripts")
    (root/"pkg-version").write_text(args[args.index("--version")+1])
    shutil.copy2(args[args.index("--component-plist")+1], root/"components.plist")
    pathlib.Path(args[-1]).write_bytes(b"TEST-PKG")
elif tool == "hdiutil":
    output = args[args.index("-o")+1] if "-o" in args else args[-1]
    pathlib.Path(output).write_bytes(b"TEST-DMG")
else:
    raise RuntimeError("unexpected fake tool "+tool)
'''


class PackagingTests(unittest.TestCase):
    def setUp(self):
        # Short path keeps fake Unix sockets below the macOS sockaddr_un limit.
        self.temporary = tempfile.TemporaryDirectory(prefix="vzpk-", dir="/tmp")
        self.root = pathlib.Path(self.temporary.name)
        self.project = self.root / "project"
        self.tools = self.root / "tools"
        self.tools.mkdir()
        (self.root / "home with spaces").mkdir()
        for name in ["stat", "dscl", "id", "sudo", "chown", "sleep", "pgrep", "helper-stop",
                     "launchctl", "ditto", "otool", "pkgbuild", "hdiutil"]:
            self.executable(self.tools / name, f"#!{sys.executable}\n{FAKE_TOOL}")
        self.env = dict(os.environ, PACKAGE_TEST_ROOT=str(self.root),
                        PATH=f"{self.tools}:{os.environ['PATH']}", RELEASE_TAG="dev")
        for path in ["scripts/package-macos.sh", "scripts/package-metadata.py",
                     "packaging/macos/preinstall", "packaging/macos/postinstall",
                     "packaging/macos/installer-common.sh", "daemon/scripts/stop-vz-helpers.sh"]:
            target = self.project / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / path, target)
        self.populate("0.0.1")

    def tearDown(self):
        self.temporary.cleanup()

    @staticmethod
    def executable(path, contents):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        path.chmod(0o755)

    def populate(self, version):
        for name in RUST_BINARIES:
            path = self.project / f"crates/{name}/Cargo.toml"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f'[package]\nname="{name}"\nversion="{version}"\n')
            self.executable(self.project / f"target/release/{name}", f"#!/bin/sh\necho '{name} {version}'\n")
        for name in SWIFT_BINARIES:
            self.executable(self.project / f"daemon/.build/release/{name}", f"#!/bin/sh\necho '{name} {version}'\n")
        swift = self.project / "daemon/Sources/VzDaemonKit/Types.swift"
        swift.parent.mkdir(parents=True, exist_ok=True)
        swift.write_text(f'public static let version = "{version}"\n')
        ui = self.project / "apps/vzctl-ui"
        (ui / "src-tauri").mkdir(parents=True, exist_ok=True)
        (ui / "src-tauri/Cargo.toml").write_text(f'[package]\nversion="{version}"\n')
        for path in [ui / "package.json", ui / "src-tauri/tauri.conf.json"]:
            path.write_text(json.dumps({"version": version}))
        app = ui / "src-tauri/target/release/bundle/macos/vzctl.app/Contents"
        app.mkdir(parents=True, exist_ok=True)
        (app / "Info.plist").write_bytes(plistlib.dumps({"CFBundleShortVersionString": version}))
        for name in ["caddy", "dex", "qemu-img"]:
            directory = self.project / f"daemon/Vendor/{name}"
            self.executable(directory / name, "#!/bin/sh\nexit 0\n")
            (directory / "VERSION").write_text("1.2.3\n")
            (directory / "LICENSE").write_text("fixture vendor license\n")
        (self.project / "LICENSE").write_text("fixture project license\n")
        (self.project / "README.md").write_text("fixture readme\n")

    def package(self, **extra):
        return subprocess.run(["sh", str(self.project / "scripts/package-macos.sh")],
                              env=dict(self.env, **extra), text=True, capture_output=True, timeout=30)

    def calls(self):
        path = self.root / "calls"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def test_payload_contains_all_required_binaries_licenses_and_hooks(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = self.root / "captured-payload/usr/local"
        for name in RUST_BINARIES + SWIFT_BINARIES:
            self.assertTrue((payload / f"bin/{name}").is_file(), name)
        for name in ["caddy", "dex", "qemu-img"]:
            for file in [name, "LICENSE", "VERSION"]:
                self.assertTrue((payload / f"libexec/vzctl/{name}/{file}").is_file())
        for hook in ["preinstall", "postinstall", "installer-common.sh", "stop-vz-helpers.sh"]:
            self.assertTrue(os.access(self.root / "captured-scripts" / hook, os.X_OK))
        archive = next((self.project / "dist").glob("*.tar.gz"))
        with tarfile.open(archive) as tar:
            files = [member.name for member in tar]
            self.assertTrue(any(name.endswith("/vzctl-mcp") for name in files))
            self.assertTrue(any(name.endswith("/libexec/caddy/LICENSE") for name in files))
            self.assertTrue(any(name.endswith("/VERSION.json") for name in files))
        for checksum in (self.project / "dist").glob("*.sha256"):
            artifact = pathlib.Path(str(checksum).removesuffix(".sha256"))
            self.assertEqual(checksum.read_text().split()[0], hashlib.sha256(artifact.read_bytes()).hexdigest())
        self.assertEqual((self.root / "pkg-version").read_text(), "0.0.1")
        components = plistlib.loads((self.root / "components.plist").read_bytes())
        self.assertEqual(components[0]["RootRelativeBundlePath"], "Applications/vzctl.app")
        self.assertIs(components[0]["BundleIsRelocatable"], False)

    def test_missing_dependency_and_mismatched_version_fail_before_packaging(self):
        binary = self.project / "target/release/vzctl-mcp"
        binary.unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("vzctl-mcp", result.stderr)
        self.assertFalse((self.root / "captured-payload").exists())
        self.populate("0.0.1")
        (self.project / "daemon/Vendor/caddy/caddy").unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("run make vendor", result.stderr)
        self.populate("0.0.1")
        result = self.package(RELEASE_TAG="v0.2.0-alpha3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match source version", result.stderr)
        (self.project / "daemon/Sources/VzDaemonKit/Types.swift").write_text('static let version = "0.0.2"')
        self.assertIn("version mismatch", self.package().stderr)

    def test_prerelease_tag_uses_numeric_pkg_version_and_qemu_must_be_relocatable(self):
        self.populate("0.1.2-alpha.3")
        result = self.package(RELEASE_TAG="v0.1.2-alpha.3", BAD_QEMU="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-relocatable", result.stderr)
        result = self.package(RELEASE_TAG="v0.1.2-alpha.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / "pkg-version").read_text(), "0.1.2")

    def test_universal_qemu_checks_dependencies_in_each_architecture(self):
        result = self.package(UNIVERSAL_QEMU="1", BAD_QEMU="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-relocatable", result.stderr)
        result = self.package(UNIVERSAL_QEMU="1")
        self.assertEqual(result.returncode, 0, result.stderr)

    def install_hooks(self):
        directory = self.root / "hooks"
        shutil.copytree(self.project / "packaging/macos", directory)
        shutil.copy2(self.tools / "helper-stop", directory / "stop-vz-helpers.sh")
        # Preserve the hook logic, redirect only its compiled-in payload locations.
        post = directory / "postinstall"
        post.write_text(post.read_text()
                        .replace("bindir=/usr/local/bin", f'bindir="{self.root}/payload/bin"')
                        .replace("libexec=/usr/local/libexec/vzctl", f'libexec="{self.root}/payload/libexec"')
                        .replace("/Applications/vzctl.app", f'"{self.root}/payload/vzctl.app"'))
        for name in RUST_BINARIES + SWIFT_BINARIES:
            self.executable(self.root / f"payload/bin/{name}", "#!/bin/sh\nexit 0\n")
        for name in ["caddy", "dex", "qemu-img"]:
            self.executable(self.root / f"payload/libexec/{name}/{name}", "#!/bin/sh\nexit 0\n")
        (self.root / "payload/vzctl.app").mkdir()
        (self.root / "loaded.json").write_text(json.dumps(["com.vzctl.supervisor", "com.vzctl.edge", "com.vzctl.net"]))
        return directory

    def run_hook(self, directory, name, **extra):
        return subprocess.run(["sh", str(directory / name)], env=dict(self.env, **extra),
                              capture_output=True, text=True, timeout=30)

    def test_install_order_is_stop_then_replace_then_start(self):
        hooks = self.install_hooks()
        before = self.run_hook(hooks, "preinstall")
        self.assertEqual(before.returncode, 0, before.stderr)
        with (self.root / "calls").open("a") as stream:
            stream.write('["payload-replacement"]\n')
        after = self.run_hook(hooks, "postinstall")
        self.assertEqual(after.returncode, 0, after.stderr)
        calls = self.calls()
        meaningful = [call for call in calls if call[0] != "launchctl" or call[1] in ("bootout", "bootstrap")]
        self.assertEqual([call[:2] for call in meaningful], [
            ["stop-vz-helpers.sh", "501"],
            ["launchctl", "bootout"], ["launchctl", "bootout"], ["launchctl", "bootout"],
            ["payload-replacement"],
            ["launchctl", "bootstrap"], ["launchctl", "bootstrap"], ["launchctl", "bootstrap"],
        ])
        stops = [call[-1] for call in calls if call[:2] == ["launchctl", "bootout"]]
        starts = [pathlib.Path(call[-1]).name for call in calls if call[:2] == ["launchctl", "bootstrap"]]
        self.assertEqual(stops, ["gui/501/com.vzctl.supervisor", "gui/501/com.vzctl.edge", "gui/501/com.vzctl.net"])
        self.assertEqual(starts, ["com.vzctl.net.plist", "com.vzctl.edge.plist", "com.vzctl.supervisor.plist"])
        self.assertFalse(any("-k" in call or "-KILL" in call for call in calls))
        plist = self.root / "home with spaces/Library/LaunchAgents/com.vzctl.net.plist"
        self.assertIn("/opt/homebrew/bin", plistlib.loads(plist.read_bytes())["EnvironmentVariables"]["PATH"])
        self.assertTrue((self.root / "home with spaces/Library/Application Support/vzctl/bin/caddy").is_file())

    def test_stop_and_start_failures_are_not_reported_as_success(self):
        hooks = self.install_hooks()
        blocked = self.run_hook(hooks, "preinstall", HELPER_RESULT="5")
        self.assertEqual(blocked.returncode, 5)
        self.assertFalse(any(call[:2] == ["launchctl", "bootout"] for call in self.calls()))
        blocked = self.run_hook(hooks, "preinstall", STOP_FAILURE="com.vzctl.supervisor")
        self.assertEqual(blocked.returncode, 5)
        self.assertIn("payload replacement refused", blocked.stderr)
        self.assertFalse(any(call[-1] == "gui/501/com.vzctl.net" for call in self.calls()))
        self.assertEqual(self.run_hook(hooks, "preinstall").returncode, 0)
        failed = self.run_hook(hooks, "postinstall", START_FAILURE="com.vzctl.edge")
        self.assertNotEqual(failed.returncode, 0)
        self.assertFalse(any(call[1:2] == ["bootstrap"] and call[-1].endswith("supervisor.plist") for call in self.calls()))

    def test_other_sessions_or_external_volume_are_rejected_without_stops(self):
        hooks = self.install_hooks()
        blocked = self.run_hook(hooks, "preinstall", CONSOLE_USER="root", PGREP_RESULT="0")
        self.assertEqual(blocked.returncode, 5)
        self.assertIn("still running", blocked.stderr)
        self.assertEqual(self.calls(), [])
        blocked = subprocess.run(["sh", str(hooks / "preinstall"), "package", "/", "/Volumes/Other"],
                                 env=self.env, capture_output=True, text=True, timeout=3)
        self.assertEqual(blocked.returncode, 5)
        self.assertIn("startup volume", blocked.stderr)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
