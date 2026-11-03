# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Exercise release version updates in an isolated Cargo project."""

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
            "package.json",
            "package-lock.json",
            "pyproject.toml",
            "bindings/embed-models/python/pyproject.toml",
            "bindings/python/warm-sdk/pyproject.toml",
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


if __name__ == "__main__":
    unittest.main()
