# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Pinned Lucene jars (Maven Central, sha256-verified) and the compiled Java comparator."""

import hashlib
import shutil
import subprocess
import urllib.request
from pathlib import Path

from .common import CACHE, ROOT

JARS = CACHE / "competitive" / "jars"
SOURCE = ROOT / "benchmarks/competitive/lucene/LuceneBm25.java"
VERSION = "10.6.0"
PREVIOUS = "9.12.3"
PINNED = {
    "10.6.0": {
        "lucene-core": "211424f423ff1afeb891095c2142a20a572e545906251f28b8faaa408d01f934",
        "lucene-analysis-common": "7e7fc5bcb7e2fd702701cfa7f5c209f28bdaf89cb8ef0ba379255c9d7c687430",
    },
    "9.12.3": {
        "lucene-core": "b64a3f8098a7572034fb30085cdee01b34ec81fb0e5a31b471536af58dc6c01b",
        "lucene-analysis-common": "fa571bd7caf0f0b4faf46a72ca004a7836f348c31d92bd522dddcc3d128d287e",
    },
}
# Lowest `javac --release` that can read each version's class files.
JAVA_RELEASE = {"10.6.0": "21", "9.12.3": "17"}


def _sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def ensure(version=VERSION):
    """Download (once) and verify the jars, compile the comparator, return the classpath."""
    JARS.mkdir(parents=True, exist_ok=True)
    paths = []
    for name, digest in PINNED[version].items():
        path = JARS / f"{name}-{version}.jar"
        if not path.exists():
            url = f"https://repo1.maven.org/maven2/org/apache/lucene/{name}/{version}/{name}-{version}.jar"
            with urllib.request.urlopen(url) as response:
                path.write_bytes(response.read())
        if _sha(path) != digest:
            raise ValueError(f"{path.name} does not match its pinned sha256")
        paths.append(path)
    classes = JARS / f"classes-{version}"
    stamp = classes / f".{_sha(SOURCE)[:16]}"
    if not stamp.exists():
        shutil.rmtree(classes, ignore_errors=True)
        classes.mkdir()
        subprocess.run(["javac", "--release", JAVA_RELEASE[version], "-cp", ":".join(map(str, paths)), "-d", str(classes), str(SOURCE)], check=True)
        stamp.write_text("")
    return ":".join([*map(str, paths), str(classes)]), paths


def footprint(version=VERSION):
    _, paths = ensure(version)
    return {"bytes": sum(p.stat().st_size for p in paths), "distributions": len(paths), "note": "two jars; also requires a JVM (not counted)"}


def java_version():
    out = subprocess.run(["java", "-version"], capture_output=True, text=True)
    return (out.stderr or out.stdout).splitlines()[0]
