# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check that a framework-adapter release set is internally consistent.

usage: check-adapter-versions.py {js,py} [--tag js-adapters-v0.1.0]

js: the six packages under integrations/js/packages and the TypeScript Claude memory tool share
one version, equal to the tag, and each adapter depends on `@writerslogic/warm-core` at `^version`.
py: the integrations package and the Python Claude memory tool share one version, equal to the
tag and to each package's `__version__`. Every package carries a copy of the root LICENSE.
Prints the version on success.
"""
import argparse
import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
JS_DIRS = [f"integrations/js/packages/{n}" for n in ("core", "langchain", "llamaindex", "mastra", "openai-agents", "vercel-ai")]
JS_DIRS.append("integrations/claude-memory-tool/typescript")
PY = {
    "integrations/python": ("warm-integrations", "src/writerslogic_warm_integrations/__init__.py"),
    "integrations/claude-memory-tool/python": ("warm-claude-tool", "src/warm_memory_tool/__init__.py"),
}
SEMVER = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?")
CORE = "@writerslogic/warm-core"


def check_license(directory: str, problems: list[str]) -> None:
    path = ROOT / directory / "LICENSE"
    if not path.is_file() or path.read_bytes() != (ROOT / "LICENSE").read_bytes():
        problems.append(f"{directory}/LICENSE must be a copy of the root LICENSE")


def js(problems: list[str]) -> list[str]:
    versions = []
    for directory in JS_DIRS:
        package = json.loads((ROOT / directory / "package.json").read_text(encoding="utf-8"))
        version = package.get("version")
        versions.append(version)
        if package.get("license") != "Apache-2.0":
            problems.append(f"{directory} must be Apache-2.0")
        if package.get("private"):
            problems.append(f"{directory} is private")
        if package.get("publishConfig", {}).get("access") != "public":
            problems.append(f"{directory} publishConfig.access must be public")
        dependency = package.get("dependencies", {}).get(CORE)
        if directory.endswith("/core") or directory.endswith("claude-memory-tool/typescript"):
            if dependency is not None:
                problems.append(f"{directory} must not depend on {CORE}")
        elif dependency != f"^{version}":
            problems.append(f"{directory} must depend on {CORE} at ^{version}, found {dependency}")
        check_license(directory, problems)
    return versions


def py(problems: list[str]) -> list[str]:
    versions = []
    for directory, (name, init) in PY.items():
        project = tomllib.loads((ROOT / directory / "pyproject.toml").read_text(encoding="utf-8"))["project"]
        if project.get("name") != name:
            problems.append(f"{directory} must be named {name}")
        version = project.get("version")
        versions.append(version)
        found = re.search(r'^__version__ = "([^"]+)"', (ROOT / directory / init).read_text(encoding="utf-8"), re.M)
        if not found or found.group(1) != version:
            problems.append(f"{directory}/{init} __version__ disagrees with pyproject.toml")
        if project.get("license") != "Apache-2.0":
            problems.append(f"{directory} must be Apache-2.0")
        check_license(directory, problems)
    return versions


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=("js", "py"))
    parser.add_argument("--tag")
    args = parser.parse_args()
    problems: list[str] = []
    versions = (js if args.kind == "js" else py)(problems)
    version = versions[0]
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        problems.append(f"version {version!r} is not semantic")
    if any(v != version for v in versions):
        problems.append(f"packages must share one version, found {versions}")
    if args.tag is not None:
        prefix = "js-adapters-v" if args.kind == "js" else "py-adapters-v"
        if args.tag != prefix + str(version):
            problems.append(f"tag {args.tag} must be {prefix}{version}")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
