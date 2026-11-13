# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exercise independent engine and portable SDK releases in an isolated tree."""

import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class VersionTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.tree = pathlib.Path(temporary.name)
        for relative in (
            "scripts/ci/bump-version.py",
            "scripts/ci/check-versions.py",
            "scripts/ci/check-sdk-versions.py",
            "package.json",
            "package-lock.json",
            "pyproject.toml",
            "bindings/embed-models/python/pyproject.toml",
            "bindings/python/warm-sdk/pyproject.toml",
            "bindings/js/warm-sdk/package.json",
            "bindings/js/warm-sdk/package-lock.json",
            "docs/server/openapi.json",
        ):
            destination = self.tree / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, destination)
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
        (self.tree / "Cargo.toml").write_text(
            f'[package]\nname = "warm-rust"\nversion = "{version}"\nedition = "2021"\n'
        )
        (self.tree / "src").mkdir()
        (self.tree / "src/lib.rs").write_text("")

    def run_script(self, name: str, *args: str) -> subprocess.CompletedProcess:
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(self.tree / "target")
        return subprocess.run(
            [sys.executable, str(self.tree / "scripts/ci" / name), *args],
            cwd=self.tree,
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_bump_keeps_the_contract_and_package_versions_consistent(self) -> None:
        path = self.tree / "docs/server/openapi.json"
        routes = json.loads(path.read_text())["paths"]
        for version in ("1.2.3", "1.2.4-rc.1"):
            with self.subTest(version=version):
                result = self.run_script("bump-version.py", version)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                contract = json.loads(path.read_text())
                self.assertEqual(contract["info"]["version"], version)
                self.assertEqual(contract["paths"], routes)
                lock = tomllib.loads((self.tree / "Cargo.lock").read_text())
                self.assertEqual(lock["package"][0]["version"], version)
                for tag in (f"v{version}", f"py-v{version}"):
                    checked = self.run_script("check-versions.py", "--tag", tag)
                    self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
                sdk = self.run_script("check-sdk-versions.py")
                self.assertEqual(sdk.returncode, 0, sdk.stdout + sdk.stderr)

    def test_stale_contract_is_rejected_before_building(self) -> None:
        path = self.tree / "docs/server/openapi.json"
        contract = json.loads(path.read_text())
        contract["info"]["version"] = "0.0.0"
        path.write_text(json.dumps(contract))
        result = self.run_script("check-versions.py")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("docs/server/openapi.json is 0.0.0", result.stderr)

    def test_stale_or_missing_npm_lock_versions_are_rejected(self) -> None:
        path = self.tree / "package-lock.json"
        original = path.read_text()
        package = json.loads((self.tree / "package.json").read_text())
        name = next(iter(package["optionalDependencies"]))
        cases = (
            ((), "version", "0.0.0"),
            (("packages", ""), "version", "0.0.0"),
            (("packages", ""), "version", None),
            (("packages", "", "optionalDependencies"), name, "0.0.0"),
            (("packages", "", "optionalDependencies"), name, None),
            (("packages", "", "optionalDependencies"), "unexpected-platform", package["version"]),
            (("packages", f"node_modules/{name}"), "version", "0.0.0"),
            (("packages",), f"node_modules/{name}", None),
            (("packages",), "", None),
        )
        for parents, key, value in cases:
            with self.subTest(parents=parents, key=key, value=value):
                lock = json.loads(original)
                target = lock
                for parent in parents:
                    target = target[parent]
                if value is None:
                    del target[key]
                else:
                    target[key] = value
                path.write_text(json.dumps(lock))
                result = self.run_script("check-versions.py")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("package-lock.json", result.stderr)

    def test_native_package_and_binary_names_cannot_drift(self) -> None:
        for relative, parents, key in (
            ("package.json", (), "name"),
            ("package.json", ("napi",), "binaryName"),
            ("package-lock.json", (), "name"),
            ("package-lock.json", ("packages", ""), "name"),
        ):
            with self.subTest(relative=relative, parents=parents, key=key):
                path = self.tree / relative
                original = path.read_text()
                manifest = json.loads(original)
                target = manifest
                for parent in parents:
                    target = target[parent]
                target[key] = "warm-sdk"
                path.write_text(json.dumps(manifest))
                result = self.run_script("check-versions.py")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("writerslogic-warm", result.stderr)
                path.write_text(original)

    def test_native_platform_family_cannot_be_renamed_consistently(self) -> None:
        package_path = self.tree / "package.json"
        package = json.loads(package_path.read_text())
        name = next(iter(package["optionalDependencies"]))
        renamed = name.replace("writerslogic-warm-", "warm-sdk-", 1)
        self.assertNotEqual(name, renamed)
        package["optionalDependencies"][renamed] = package["optionalDependencies"].pop(name)
        package_path.write_text(json.dumps(package))
        lock_path = self.tree / "package-lock.json"
        lock = json.loads(lock_path.read_text())
        dependencies = lock["packages"][""]["optionalDependencies"]
        dependencies[renamed] = dependencies.pop(name)
        lock["packages"][f"node_modules/{renamed}"] = lock["packages"].pop(f"node_modules/{name}")
        lock_path.write_text(json.dumps(lock))
        result = self.run_script("check-versions.py")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("package family and napi.targets", result.stderr)

    def test_sdk_versions_and_shared_tag_match_independently_of_engine(self) -> None:
        version = json.loads((self.tree / "bindings/js/warm-sdk/package.json").read_text())["version"]
        (self.tree / "Cargo.toml").write_text('[package]\nversion = "99.98.97"\n')
        for arguments in ((), ("--tag", f"sdk-v{version}"), ("--tag", f"refs/tags/sdk-v{version}")):
            with self.subTest(arguments=arguments):
                result = self.run_script("check-sdk-versions.py", *arguments)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), {"version": version, "problems": 0})

    def test_sdk_tag_requires_exact_version_and_sdk_prefix(self) -> None:
        version = json.loads((self.tree / "bindings/js/warm-sdk/package.json").read_text())["version"]
        for tag in ("sdk-v99.98.97", f"v{version}", f"py-v{version}", f"sdk-v{version}-extra", ""):
            with self.subTest(tag=tag):
                result = self.run_script("check-sdk-versions.py", "--tag", tag)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout)["problems"], 1)
                self.assertIn("sdk-v prefix", result.stderr)

    def test_sdk_manifest_names_and_versions_cannot_drift(self) -> None:
        for relative, parents in (
            ("bindings/js/warm-sdk/package.json", ()),
            ("bindings/js/warm-sdk/package-lock.json", ()),
            ("bindings/js/warm-sdk/package-lock.json", ("packages", "")),
            ("bindings/python/warm-sdk/pyproject.toml", ("project",)),
        ):
            path = self.tree / relative
            original = path.read_text()
            for key, replacement in (("name", "writerslogic-warm"), ("version", "99.98.97")):
                with self.subTest(relative=relative, parents=parents, key=key):
                    if relative.endswith(".toml"):
                        manifest = tomllib.loads(original)
                        path.write_text(original.replace(f'{key} = "{manifest["project"][key]}"', f'{key} = "{replacement}"', 1))
                    else:
                        manifest = json.loads(original)
                        target = manifest
                        for parent in parents:
                            target = target[parent]
                        target[key] = replacement
                        path.write_text(json.dumps(manifest))
                    result = self.run_script("check-sdk-versions.py")
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertGreater(json.loads(result.stdout)["problems"], 0)
                    path.write_text(original)

    def test_sdk_runtime_dependencies_are_rejected_in_manifest_and_lock(self) -> None:
        for relative, parents in (
            ("bindings/js/warm-sdk/package.json", ()),
            ("bindings/js/warm-sdk/package-lock.json", ("packages", "")),
        ):
            path = self.tree / relative
            original = path.read_text()
            for field in ("dependencies", "optionalDependencies", "peerDependencies"):
                with self.subTest(relative=relative, field=field):
                    manifest = json.loads(original)
                    target = manifest
                    for parent in parents:
                        target = target[parent]
                    target[field] = {"writerslogic-warm": "0.6.7"}
                    path.write_text(json.dumps(manifest))
                    result = self.run_script("check-sdk-versions.py")
                    self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                    self.assertIn(f"must not declare {field}", result.stderr)
            path.write_text(original)

    def test_sdk_install_hooks_and_unreadable_lock_fail_closed(self) -> None:
        path = self.tree / "bindings/js/warm-sdk/package.json"
        original = path.read_text()
        for hook in ("preinstall", "install", "postinstall"):
            with self.subTest(hook=hook):
                manifest = json.loads(original)
                manifest.setdefault("scripts", {})[hook] = "node install.cjs"
                path.write_text(json.dumps(manifest))
                result = self.run_script("check-sdk-versions.py")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("install lifecycle hooks", result.stderr)
        path.write_text(original)
        lock = self.tree / "bindings/js/warm-sdk/package-lock.json"
        for invalid in ("[", "[]", None):
            with self.subTest(invalid=invalid):
                if invalid is None:
                    lock.unlink()
                else:
                    lock.write_text(invalid)
                result = self.run_script("check-sdk-versions.py")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertGreater(json.loads(result.stdout)["problems"], 0)
                self.assertIn("must be a readable manifest object", result.stderr)

    def test_sdk_bundled_dependencies_and_locked_install_script_are_rejected(self) -> None:
        for relative, parents, field, value in (
            ("bindings/js/warm-sdk/package.json", (), "bundleDependencies", True),
            ("bindings/js/warm-sdk/package.json", (), "bundledDependencies", ["native-engine"]),
            ("bindings/js/warm-sdk/package-lock.json", ("packages", ""), "hasInstallScript", True),
        ):
            with self.subTest(relative=relative, field=field):
                path = self.tree / relative
                original = path.read_text()
                manifest = json.loads(original)
                target = manifest
                for parent in parents:
                    target = target[parent]
                target[field] = value
                path.write_text(json.dumps(manifest))
                result = self.run_script("check-sdk-versions.py")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertGreater(json.loads(result.stdout)["problems"], 0)
                path.write_text(original)


if __name__ == "__main__":
    unittest.main()
