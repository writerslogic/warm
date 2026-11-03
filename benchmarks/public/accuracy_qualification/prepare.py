# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Freeze and extract untouched public accuracy inputs; never run search."""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import time
import urllib.request
import urllib.parse

import h5py
import numpy as np


HERE = Path(__file__).resolve().parent


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    with path.open("x", encoding="utf8") as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write("\n")


def extract(dataset, count, dimensions, output):
    if dataset.ndim != 2 or dataset.shape[1] != dimensions or dataset.shape[0] < count:
        raise ValueError("unexpected input shape")
    retained, skipped, written = [], [], 0
    with output.open("xb") as stream:
        for start in range(0, dataset.shape[0], 2048):
            with np.errstate(over="ignore", invalid="ignore"):
                batch = np.asarray(dataset[start:start + 2048], dtype="<f4")
            finite = np.isfinite(batch).all(axis=1)
            nonzero = np.any(batch != 0, axis=1)
            for offset, row in enumerate(batch):
                ordinal = start + offset
                if not finite[offset] or not nonzero[offset]:
                    skipped.append({"ordinal": ordinal, "reason": "nonfinite-f32" if not finite[offset] else "zero-cosine-vector"})
                    continue
                stream.write(row.tobytes())
                retained.append(ordinal)
                written += 1
                if written == count:
                    stream.flush()
                    return {"sourceShape": list(dataset.shape), "retainedOrdinals": retained,
                            "skipped": skipped, "sha256": digest(output), "bytes": output.stat().st_size}
    raise ValueError("not enough eligible input vectors")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir()
    plan = json.loads((HERE / "PLAN.json").read_text())
    for name in ["PLAN.json", "prepare.py", "pyproject.toml", "uv.lock"]:
        (root / name).write_bytes((HERE / name).read_bytes())
    write(root / "freeze.json", {"plan": plan, "sources": {name: digest(root / name) for name in
          ["PLAN.json", "prepare.py", "pyproject.toml", "uv.lock"]}, "python": platform.python_version(),
          "numpy": np.__version__, "h5py": h5py.__version__, "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())})
    completed = []
    try:
        for spec in plan["datasets"]:
            directory = root / spec["name"]
            directory.mkdir()
            download = directory / "source.hdf5.partial"
            request = urllib.request.Request(spec["url"], headers={"User-Agent": "Mozilla/5.0 WARM accuracy qualification"})
            start, total = time.monotonic(), 0
            with urllib.request.urlopen(request, timeout=60) as response, download.open("xb") as stream:
                resolved = urllib.parse.urlsplit(response.url)
                final_url = urllib.parse.urlunsplit((resolved.scheme, resolved.netloc, resolved.path, "", ""))
                while chunk := response.read(1024 * 1024):
                    total += len(chunk)
                    if total > spec["maxDownloadBytes"] or time.monotonic() - start > plan["limits"]["downloadSecondsPerDataset"]:
                        raise RuntimeError("frozen download limit exceeded")
                    stream.write(chunk)
            source = directory / "source.hdf5"
            download.rename(source)
            with h5py.File(source, "r") as data:
                metric = data.attrs.get("distance")
                if isinstance(metric, bytes):
                    metric = metric.decode()
                if metric != "angular":
                    raise ValueError("published input is not angular")
                training = extract(data["train"], plan["inputSelection"]["rows"], spec["dimensions"], directory / "train.f32")
                queries = extract(data["test"], plan["inputSelection"]["queries"], spec["dimensions"], directory / "test.f32")
            record = {**spec, "sourceSha256": digest(source), "downloadBytes": total,
                      "downloadSeconds": time.monotonic() - start, "finalUrl": final_url,
                      "training": training, "queries": queries}
            write(directory / "inputs.json", record)
            completed.append({"name": spec["name"], "inputsSha256": digest(directory / "inputs.json")})
            print(json.dumps({"prepared": spec["name"], "rows": len(training["retainedOrdinals"]),
                              "queries": len(queries["retainedOrdinals"]), "skipped": len(training["skipped"]) + len(queries["skipped"])}), flush=True)
    except Exception as error:
        write(root / "finished.json", {"complete": False, "completed": completed, "error": {"type": type(error).__name__, "message": str(error)}})
        raise
    write(root / "finished.json", {"complete": True, "completed": completed, "freezeSha256": digest(root / "freeze.json")})


if __name__ == "__main__":
    main()
