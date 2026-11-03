# SPDX-License-Identifier: Apache-2.0
"""Issue #165 at full scale: query-reinforced edge repair on glove-100-angular and
sift-128-euclidean, on one Modal x86 container.

  modal volume put warm-ann-confirm ~/.cache/hms-bench/glove-100-angular /glove-100-angular    # once
  modal volume put warm-ann-confirm ~/.cache/hms-bench/sift-128-euclidean /sift-128-euclidean  # once
  uv run --with modal modal run --detach benchmarks/public/ann_modal_repair.py --rev HEAD --cap-usd 8

Runs the `qgraph-repair-bench` binary (built from `git archive <rev>`) over the whole base of
each set: one base graph, every arm derived from it, test.f32 split 5,000/5,000 by seed 165
(construction / evaluation). The container's cores are shared hardware threads: paired ratios
between arms are the result, absolute QPS is indicative.
"""
from __future__ import annotations

import json
import subprocess
import time
from pathlib import Path

import modal

REPO = Path(__file__).resolve().parent.parent.parent
CPU, MEM_GIB = 8, 32
USD_PER_HOUR = CPU * 0.0473 + MEM_GIB * 0.008  # as in ann_modal_graph.py
SETS = {
    "glove-100-angular": ["--dim", "100", "--metric", "angular", "--n", "1183514"],
    "sift-128-euclidean": ["--dim", "128", "--metric", "euclidean", "--n", "1000000"],
}

VOL = modal.Volume.from_name("warm-ann-confirm", create_if_missing=True)
app = modal.App("warm-ann-repair")
image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("curl", "build-essential", "pkg-config", "time")
    .run_commands("curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.96.0")
)


@app.function(image=image, volumes={"/vol": VOL}, cpu=CPU, memory=MEM_GIB * 1024, timeout=6 * 3600)
def run(archive: bytes, lock: bytes, cap_usd: float) -> dict:
    started = time.time()
    env = {"PATH": "/root/.cargo/bin:/usr/bin:/bin", "HOME": "/root", "CARGO_BUILD_JOBS": str(CPU)}
    src = Path("/work/tip")
    src.mkdir(parents=True, exist_ok=True)
    (src / "src.tar").write_bytes(archive)
    subprocess.run(["tar", "xf", "src.tar"], cwd=src, check=True)
    (src / "Cargo.lock").write_bytes(lock)
    target = Path("/vol/target-repair")
    subprocess.run(["cargo", "build", "--release", "--features", "cli", "--bin", "qgraph-repair-bench"],
                   cwd=src, env=env | {"CARGO_TARGET_DIR": str(target)}, check=True)
    out = Path("/vol/repair-out")
    out.mkdir(exist_ok=True)
    rows = []
    for name, extra in SETS.items():
        if (time.time() - started) / 3600 * USD_PER_HOUR > cap_usd * 0.9:
            rows.append({"set": name, "skipped": "spend cap"})
            continue
        result = out / f"{name}.json"
        cmd = ["/usr/bin/time", "-v", str(target / "release" / "qgraph-repair-bench"), "--data", f"/vol/{name}",
               "--cache", f"/vol/repair-{name}.graph", "--out", str(result), *extra]
        proc = subprocess.run(cmd, text=True, capture_output=True, env=env)
        peak = next((int(l.split(":")[1]) for l in proc.stderr.splitlines() if "Maximum resident" in l), None)
        rows.append({"set": name, "exit": proc.returncode, "peak_rss_kib": peak, "stderr_tail": proc.stderr[-2000:],
                     "result": json.loads(result.read_text()) if result.exists() else None})
        VOL.commit()
    return {"rows": rows, "wall_s": round(time.time() - started, 1),
            "spent_usd_estimate": round((time.time() - started) / 3600 * USD_PER_HOUR, 2)}


@app.local_entrypoint()
def main(rev: str = "HEAD", cap_usd: float = 8.0, out: str = "benchmarks/results/qgraph_repair_v1/modal_full.raw.json"):
    archive = subprocess.run(["git", "archive", "--format=tar", rev], cwd=REPO, check=True, capture_output=True).stdout
    report = run.remote(archive, (REPO / "Cargo.lock").read_bytes(), cap_usd)
    Path(out).write_text(json.dumps(report, indent=1))
    print(json.dumps({k: report[k] for k in ("wall_s", "spent_usd_estimate")}))
