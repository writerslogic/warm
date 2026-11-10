# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Fetch the fixed E42 prefix and two pinned, public counting dependencies."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import urllib.request


REVISION = "98d7416c24c778c2fee6e6f3006e7a073259d48f"
TOTAL = 2737100077
CAP = 33554432
URL = f"https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/{REVISION}/longmemeval_m_cleaned.json"


def fetch(url, destination, cap, expected=None, ranged=False):
    headers = {"Accept-Encoding": "identity"}
    if ranged:
        headers["Range"] = f"bytes=0-{cap - 1}"
    request = urllib.request.Request(url, headers=headers)
    digest = hashlib.sha256()
    total = 0
    partial = destination.with_name(destination.name + ".partial")
    if destination.exists() or partial.exists():
        raise ValueError("Refusing to overwrite prior fetch evidence")
    with urllib.request.urlopen(request, timeout=60) as response:
        encoding = response.headers.get("Content-Encoding", "identity")
        if encoding != "identity":
            raise ValueError("Unexpected encoded response")
        if ranged:
            match = re.fullmatch(r"bytes (\d+)-(\d+)/(\d+)", response.headers.get("Content-Range", ""))
            if response.status != 206 or not match or tuple(map(int, match.groups())) != (0, cap - 1, TOTAL):
                raise ValueError("Server did not honor the exact bounded range")
        elif response.status != 200:
            raise ValueError("Unexpected HTTP status")
        receipt = {"source_url": url, "http_status": response.status,
                   "content_range": response.headers.get("Content-Range"),
                   "content_encoding": encoding,
                   "retrieved_at": datetime.now(timezone.utc).isoformat()}
        with partial.open("xb") as stream:
            while block := response.read(min(1024 * 1024, cap + 1 - total)):
                total += len(block)
                if total > cap:
                    raise ValueError("Response exceeded the fixed allocation limit")
                stream.write(block)
                digest.update(block)
    if (ranged and total != cap) or (expected and digest.hexdigest() != expected):
        raise ValueError("Pinned input length or hash mismatch")
    partial.rename(destination)
    receipt.update(bytes=total, sha256=digest.hexdigest(), path=str(destination.resolve()))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=False)
    receipt = {"experiment": "E42", "revision": REVISION,
               "requested_range": {"start": 0, "end": CAP - 1},
               "full_file_sha256_verified": False,
               "scope": "Fixed first32MiB of pinned M release; no full-file verification or score."}
    try:
        receipt["prefix"] = fetch(URL, args.output_dir / "M-prefix.bin", CAP, ranged=True)
        receipt["generation"] = fetch(
            "https://raw.githubusercontent.com/xiaowu0162/LongMemEval/9e0b455f4ef0e2ab8f2e582289761153549043fc/src/generation/run_generation.py",
            args.output_dir / "run_generation.source", 1024 * 1024,
            "4f1eb3c69d7ad40f04065b9c0bc86f6582441018fc6ff751d162d66c95baf672")
        receipt["tokenizer"] = fetch(
            "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken",
            args.output_dir / "o200k_base.tiktoken", 8 * 1024 * 1024,
            "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d")
        receipt["decision"] = "accept-pinned-counting-inputs"
    except Exception as error:
        receipt.update(decision="reject-fetch", failure_type=type(error).__name__, failure=str(error))
        raise
    finally:
        (args.output_dir / "fetch-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
