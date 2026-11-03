#!/usr/bin/env python3
"""Interleaved postings block-size sweep over the competitive_text binary (build + serve phases).

usage: sweep.py BIN_DIR OUT_JSON WORK_DIR REPEATS DATASET[,DATASET...] SIZE[,SIZE...]
binaries: BIN_DIR/competitive_text-<SIZE>
"""
import hashlib, json, os, re, shutil, subprocess, sys, time
from pathlib import Path

DATA = Path.home() / ".cache/warm-bench/competitive/data"
FOLDERS = {"memory": "text-memory-lme-dev", "nfcorpus": "text-nfcorpus-dev", "scifact": "text-scifact-train"}


def run(cmd):
    t = time.perf_counter()
    p = subprocess.run(["/usr/bin/time", "-l", *cmd], capture_output=True, text=True, env={**os.environ, "RAYON_NUM_THREADS": "1"})
    wall = time.perf_counter() - t
    if p.returncode:
        raise SystemExit(f"{cmd}\n{p.stderr[-800:]}")
    m = re.search(r"(\d+)\s+maximum resident set size", p.stderr)
    return wall, int(m.group(1)) if m else None


def disk(path):
    total = 0
    for root, _, files in os.walk(path):
        for f in files:
            total += os.lstat(os.path.join(root, f)).st_blocks * 512
    return total


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(q * len(xs)))]


def main():
    bins, out, work, repeats, datasets, sizes = sys.argv[1:7]
    bins, work, repeats = Path(bins), Path(work), int(repeats)
    datasets, sizes = datasets.split(","), sizes.split(",")
    result = {"sizes": sizes, "repeats": repeats, "datasets": {}}
    for ds in datasets:
        folder = DATA / FOLDERS[ds]
        serve_in = folder / "serve-input.json"
        d = result["datasets"][ds] = {s: {"build": [], "serve": []} for s in sizes}
        digests = {}
        stores = {}
        for r in range(repeats):
            order = sizes[r % len(sizes):] + sizes[:r % len(sizes)]
            for s in order:
                store = work / f"{ds}-{s}"
                shutil.rmtree(store, ignore_errors=True)
                store.parent.mkdir(parents=True, exist_ok=True)
                o = work / f"{ds}-{s}.build.json"
                wall, rss = run([str(bins / f"competitive_text-{s}"), "build", "--input", str(folder / "input.json"),
                                 "--store", str(store), "--out", str(o), "--t0-ns", "0", "--encode", "lexical"])
                body = json.loads(o.read_text())
                d[s]["build"].append({"ingest_s": body["ingest_s"], "wall_s": wall, "rss": rss, "disk": disk(store)})
                stores[s] = store
        for r in range(repeats):
            order = sizes[r % len(sizes):] + sizes[:r % len(sizes)]
            for s in order:
                o = work / f"{ds}-{s}.serve.json"
                wall, rss = run([str(bins / f"competitive_text-{s}"), "serve", "--input", str(serve_in), "--store", str(stores[s]),
                                 "--out", str(o), "--t0-ns", str(time.time_ns()), "--passes", "3"])
                body = json.loads(o.read_text())
                passes = body["passes_ns"]
                lat = [x for p in passes for x in p]
                qps = sorted(len(p) / (sum(p) / 1e9) for p in passes)[len(passes) // 2]
                if r == 0:
                    digests[s] = hashlib.sha256(json.dumps(body["rows"], sort_keys=True).encode()).hexdigest()
                d[s]["serve"].append({"qps": qps, "p50_us": pct(lat, .5) / 1e3, "p99_us": pct(lat, .99) / 1e3, "rss": rss,
                                      "open_s": body["open_s"], "queries": len(passes[0])})
        result["datasets"][ds]["_result_digests"] = digests
        print(ds, "digests identical:", len(set(digests.values())) == 1, flush=True)
    Path(out).write_text(json.dumps(result, indent=1))


main()
