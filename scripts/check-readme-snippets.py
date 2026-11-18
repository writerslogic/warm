# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0

"""Runs the README code blocks marked `quickstart-check` so the README cannot rot.

A fenced block opened with ```js quickstart-check is run with Node from the repository root.
A block opened with ```rust quickstart-check is a complete program (its own `fn main`); it is
written verbatim as a bin of the quickstart crate (examples/quickstart/rust) and run with cargo.
`cargo test` also compiles and runs it as a doctest of README.md (src/lib.rs).

Integration docs (docs/integrations/**/*.md and integrations/**/README.md) mark a block with an HTML
comment on the line before the fence, so the rendered page keeps a plain code fence:

    <!-- quickstart-check: python langchain -->        run with `uv run --extra langchain` in integrations/python
    <!-- quickstart-check: python langchain,ag2 -->    several extras (use `-` for none)
    <!-- quickstart-check: skip: requires API key -->  not run; the reason is printed

`docs` mode starts a fresh warm-server (WARM_SERVER_BIN, `--no-auth --input-dim 384` on 127.0.0.1:8080)
for each Python block, so the blocks run exactly as written.

Usage: python scripts/check-readme-snippets.py [js|rust|all|docs [path-substring]]
"""

import os
import re
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FENCE = re.compile(r"^```(\w+) quickstart-check\n(.*?)^```$", re.S | re.M)
MARKED = re.compile(r"^<!--\s*quickstart-check:\s*(.+?)\s*-->[ \t]*\n+```(\w+)[^\n]*\n(.*?)^```$", re.S | re.M)
DOC_GLOBS = ("docs/integrations/**/*.md", "integrations/**/README.md")
SERVER_ADDR = ("127.0.0.1", 8080)


def blocks(text: str, language: str) -> list[str]:
    return [body for lang, body in FENCE.findall(text) if lang == language]


def run_js(body: str) -> None:
    with tempfile.NamedTemporaryFile("w", suffix=".cjs", dir=ROOT, delete=False) as handle:
        handle.write(body)
    try:
        subprocess.run(["node", handle.name], cwd=ROOT, check=True)
    finally:
        Path(handle.name).unlink()


def run_rust(body: str) -> None:
    crate = ROOT / "examples" / "quickstart" / "rust"
    target = crate / "src" / "bin" / "readme_snippet.rs"
    target.parent.mkdir(exist_ok=True)
    target.write_text(body)
    try:
        subprocess.run(
            ["cargo", "run", "--release", "--manifest-path", str(crate / "Cargo.toml"), "--bin", "readme_snippet"],
            cwd=ROOT,
            check=True,
        )
    finally:
        target.unlink()
        if not any(target.parent.iterdir()):
            target.parent.rmdir()


def doc_blocks(text: str) -> list[tuple[str, str, str]]:
    """(spec, language, body) for every block preceded by a quickstart-check comment."""
    return [(spec, lang, body) for spec, lang, body in MARKED.findall(text)]


def doc_files() -> list[Path]:
    found = {path for pattern in DOC_GLOBS for path in ROOT.glob(pattern) if "node_modules" not in path.parts}
    return sorted(found)


class WarmServer:
    """A throwaway warm-server on the address the docs use; one per block so no state leaks between pages."""

    def __enter__(self):
        binary = os.environ.get("WARM_SERVER_BIN")
        if not binary or not Path(binary).exists():
            raise SystemExit("set WARM_SERVER_BIN to a warm-server binary (cargo build --release --features server --bin warm-server)")
        with socket.socket() as probe:
            if probe.connect_ex(SERVER_ADDR) == 0:
                raise SystemExit(f"{SERVER_ADDR[0]}:{SERVER_ADDR[1]} is already in use; stop it first")
        self.data = tempfile.TemporaryDirectory()
        self.proc = subprocess.Popen(
            [binary, "--bind", "%s:%d" % SERVER_ADDR, "--no-auth", "--input-dim", "384", "--dim", "1024", "--data-dir", self.data.name],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        line = self.proc.stdout.readline()
        if "listening on" not in line:
            self.proc.kill()
            raise SystemExit(f"warm-server did not start: {line!r}")
        return self

    def __exit__(self, *exc):
        self.proc.terminate()
        self.proc.wait(timeout=30)
        self.data.cleanup()


def run_doc_python(spec: str, body: str) -> None:
    extras = [] if spec == "-" else spec.split(",")
    command = ["uv", "run", "--isolated", "--python", "3.12"]
    for extra in extras:
        command += ["--extra", extra]
    with tempfile.TemporaryDirectory() as scratch, WarmServer():
        script = Path(scratch) / "quickstart.py"
        script.write_text(body)
        subprocess.run([*command, "python", str(script)], cwd=ROOT / "integrations" / "python", check=True)


def run_docs(only: str = "") -> int:
    ran = skipped = 0
    for path in (p for p in doc_files() if only in str(p)):
        for spec, lang, body in doc_blocks(path.read_text()):
            where = path.relative_to(ROOT)
            if spec.startswith("skip:"):
                skipped += 1
                print(f"SKIP {where}: {spec[5:].strip()}")
                continue
            kind, _, rest = spec.partition(" ")
            if kind != lang or lang != "python":
                raise SystemExit(f"{where}: unsupported marker {spec!r} on a {lang} block")
            print(f"RUN  {where} ({rest})", flush=True)
            run_doc_python(rest.strip(), body)
            ran += 1
    print(f"docs quickstarts: {ran} ran, {skipped} skipped")
    return 0 if ran else 1


class ExtractionTests(unittest.TestCase):
    def test_only_marked_blocks_of_the_language_are_extracted(self):
        text = "```js\nskip()\n```\n```js quickstart-check\nrun()\n```\n```rust quickstart-check\nfn main() {}\n```\n"
        self.assertEqual(blocks(text, "js"), ["run()\n"])
        self.assertEqual(blocks(text, "rust"), ["fn main() {}\n"])

    def test_comment_marked_blocks(self):
        text = "<!-- quickstart-check: python a,b -->\n\n```python\nrun()\n```\n```python\nplain()\n```\n<!-- quickstart-check: skip: requires API key -->\n```ts\nx()\n```\n"
        self.assertEqual(doc_blocks(text), [("python a,b", "python", "run()\n"), ("skip: requires API key", "ts", "x()\n")])


def main() -> int:
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    if which == "docs":
        return run_docs(sys.argv[2] if len(sys.argv) > 2 else "")
    text = (ROOT / "README.md").read_text()
    found = 0
    for language, runner in (("js", run_js), ("rust", run_rust)):
        if which in (language, "all"):
            for body in blocks(text, language):
                found += 1
                runner(body)
    if found == 0:
        print("no quickstart-check blocks found in README.md", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
