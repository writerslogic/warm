# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Check the contents of packed adapter artifacts before they can be published.

usage: verify-adapter-artifacts.py npm FILE.tgz...
       verify-adapter-artifacts.py wheel FILE.whl...
       verify-adapter-artifacts.py sdist FILE.tar.gz...

npm: only dist/, README.md, LICENSE and package.json; no tests, sources, maps, env files or keys;
every `main`, `module`, `types` and `exports` target exists; LICENSE matches the root copy.
wheel/sdist: no tests, examples or secrets; LICENSE and a long description are present; the
integrations wheel carries its PEP 561 marker.
"""
import json
import pathlib
import re
import sys
import tarfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
FORBIDDEN = re.compile(
    r"(^|/)(tests?|__tests__|examples?|node_modules|\.git|\.github)(/|$)|\.(map|env|pem|key|p12|tgz|whl)$|(^|/)\.env|(^|/)(id_rsa|secrets?)(\.|$)|\.test\.|_test\.py$",
    re.I,
)
NPM_ALLOWED = re.compile(r"^(dist/.+|README\.md|LICENSE|package\.json)$")


def targets(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from targets(item)


def npm(path: pathlib.Path, problems: list[str]) -> None:
    with tarfile.open(path) as archive:
        members = {m.name: m for m in archive.getmembers() if m.isfile()}
        names = {n.removeprefix("package/") for n in members}
        for name in sorted(names):
            if FORBIDDEN.search(name) or not NPM_ALLOWED.fullmatch(name):
                problems.append(f"{path.name}: unexpected file {name}")
        for required in ("README.md", "LICENSE", "package.json"):
            if required not in names:
                problems.append(f"{path.name}: missing {required}")
        if "package.json" not in names:
            return
        manifest = json.load(archive.extractfile(members["package/package.json"]))
        license_member = members.get("package/LICENSE")
        if license_member and archive.extractfile(license_member).read() != (ROOT / "LICENSE").read_bytes():
            problems.append(f"{path.name}: LICENSE differs from the root LICENSE")
        for field in ("name", "version", "description", "license", "repository", "homepage", "bugs", "keywords"):
            if not manifest.get(field):
                problems.append(f"{path.name}: package.json lacks {field}")
        if manifest.get("license") != "Apache-2.0":
            problems.append(f"{path.name}: license must be Apache-2.0")
        if not manifest.get("types"):
            problems.append(f"{path.name}: package.json lacks types")
        entries = [manifest.get(f) for f in ("main", "module", "types")] + list(targets(manifest.get("exports", {})))
        for entry in entries:
            if entry and entry.removeprefix("./") not in names:
                problems.append(f"{path.name}: {entry} is not in the tarball")
        for scripts in ("preinstall", "install", "postinstall"):
            if scripts in manifest.get("scripts", {}):
                problems.append(f"{path.name}: must not define {scripts}")


def python(path: pathlib.Path, kind: str, problems: list[str]) -> None:
    if kind == "wheel":
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            metadata = next((archive.read(n).decode() for n in names if n.endswith(".dist-info/METADATA")), "")
    else:
        with tarfile.open(path) as archive:
            names = [m.name.split("/", 1)[-1] for m in archive.getmembers() if m.isfile()]
            metadata = next((archive.extractfile(m).read().decode() for m in archive.getmembers() if m.name.endswith("/PKG-INFO")), "")
    for name in names:
        if FORBIDDEN.search(name.split("/", 1)[-1] if kind == "sdist" else name):
            problems.append(f"{path.name}: unexpected file {name}")
    if not any(re.search(r"(^|/)LICENSE$", n) for n in names):
        problems.append(f"{path.name}: missing LICENSE")
    if "License-Expression: Apache-2.0" not in metadata:
        problems.append(f"{path.name}: metadata license is not Apache-2.0")
    if not re.search(r"\n\n\S", metadata):
        problems.append(f"{path.name}: metadata has no long description (README)")
    if kind == "wheel" and "integrations" in path.name and "writerslogic_warm_integrations/py.typed" not in names:
        problems.append(f"{path.name}: missing py.typed")


def main() -> int:
    if len(sys.argv) < 3 or sys.argv[1] not in ("npm", "wheel", "sdist"):
        print(__doc__, file=sys.stderr)
        return 2
    kind = sys.argv[1]
    problems: list[str] = []
    for argument in sys.argv[2:]:
        path = pathlib.Path(argument)
        npm(path, problems) if kind == "npm" else python(path, kind, problems)
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"{len(sys.argv) - 2} {kind} artifact(s) verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
