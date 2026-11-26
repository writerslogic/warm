# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Merge the per-platform native binding SBOMs into the npm package SBOM.

usage: merge-sboms.py <package.cdx.json> <binding.cdx.json>... > <merged.cdx.json>

warm-memory ships every platform binding in one tarball, so its SBOM must list the Rust
components of all of them. Components and dependency edges are de-duplicated by bom-ref;
each binding's own top-level component becomes a component of the package.
"""
import json
import sys


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[0]) as handle:
        merged = json.load(handle)
    components = {c.get("bom-ref") or json.dumps(c, sort_keys=True): c for c in merged.get("components", [])}
    dependencies = {d["ref"]: set(d.get("dependsOn", [])) for d in merged.get("dependencies", [])}
    for path in argv[1:]:
        with open(path) as handle:
            sbom = json.load(handle)
        if sbom.get("bomFormat") != "CycloneDX":
            print(f"{path} is not a CycloneDX SBOM", file=sys.stderr)
            return 1
        top = sbom.get("metadata", {}).get("component")
        for component in ([top] if top else []) + sbom.get("components", []):
            components.setdefault(component.get("bom-ref") or json.dumps(component, sort_keys=True), component)
        for dependency in sbom.get("dependencies", []):
            dependencies.setdefault(dependency["ref"], set()).update(dependency.get("dependsOn", []))
    merged["components"] = sorted(components.values(), key=lambda c: (c.get("bom-ref", ""), c.get("name", "")))
    merged["dependencies"] = [
        {"ref": ref, "dependsOn": sorted(depends)} for ref, depends in sorted(dependencies.items())
    ]
    json.dump(merged, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
