# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""JavaScript comparison: WARM (`warm-memory`) against Vectra, both called in-process from Node.

Every system indexes the same document vectors and answers with the same query vectors (embedded once by WARM's local
embedder, `competitive_hybrid embed`), so the comparison is the stores, not the models. The Node worker
(`benchmarks/competitive/js/vectra_bench.mjs`) runs one phase per process: a build repeat or one serve round.

Rows (`ROWS`): dense exact cosine (`vectra`, `warm-vector`), Vectra's keyword option (`vectra-bm25`, a bounded query sample:
it rebuilds a BM25 engine per query), WARM hybrid and WARM lexical-only (`warm-hybrid`, `warm-lexical`, which share one
store, built as `warm-docs`).
"""

import hashlib
import json
import os
import platform
import random
import shutil
import statistics
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

from . import common, textdata
from .common import CACHE, ROOT, log

JS = ROOT / "benchmarks/competitive/js"
WORKER = JS / "vectra_bench.mjs"
WORK = CACHE / "competitive" / "js"
K = 10
ROUNDS = 5
BUILD_REPEATS = 3
BM25_QUERIES = int(os.environ.get("WARM_BENCH_JS_BM25_QUERIES", "10"))  # 0 skips the row; default Vectra's BM25 path rebuilds its engine per query and exhausts the heap after about 20 queries
DATASETS = {"scifact": "beir:scifact", "nfcorpus": "beir:nfcorpus", "fiqa": "beir:fiqa", "memory": "memory"}

# row -> (display, store it reads, contract)
ROWS = {
    "vectra": ("Vectra 0.15.0 exact cosine", "vectra", "dense"),
    "warm-vector": ("WARM vector records, exact", "warm-vector", "dense"),
    "vectra-bm25": ("Vectra isBm25 (appended BM25 list)", "vectra-bm25", "hybrid"),
    "warm-hybrid": ("WARM hybrid (BM25 + cosine, blend)", "warm-docs", "hybrid"),
    "warm-lexical": ("WARM lexical only", "warm-docs", "keyword"),
}
STORES = ["vectra", "vectra-bm25", "warm-vector", "warm-docs"]
UNSUPPORTED = {
    "vectra": {"keyword": "Vectra 0.15.0 has no keyword-only query: `queryItems(vector, query, topK, filter, isBm25)` always runs the vector scan "
                          "first and `isBm25` only appends BM25 hits (from `wink-bm25-text-search`) after the semantic list; there is no fusion."},
}


def versions():
    def pkg(name):
        return json.loads((JS / "node_modules" / name / "package.json").read_text())["version"]
    node = subprocess.run(["node", "--version"], capture_output=True, text=True, check=True).stdout.strip()
    warm = pkg("warm-memory")
    if os.environ.get("WARM_MEMORY_DIR"):
        warm = json.loads((Path(os.environ["WARM_MEMORY_DIR"]) / "package.json").read_text())["version"] + " (built from source)"
    return {"node": node, "vectra": pkg("vectra"), "warm-memory": warm}


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prepare(dataset, embed_bin):
    """Documents and queries of the lexical text protocol, embedded once; returns the prepared directory."""
    folder = textdata.load(DATASETS[dataset])
    out = WORK / dataset
    out.mkdir(parents=True, exist_ok=True)
    input_json = json.loads((folder / "input.json").read_text())
    stamp = sha256(folder / "input.json")
    if (out / "input.sha").exists() and (out / "input.sha").read_text() == stamp:
        return out
    docs, queries = input_json["documents"], input_json["queries"]
    (out / "corpus.json").write_text(json.dumps({"ids": [d["id"] for d in docs], "texts": [d["text"] for d in docs]}))
    (out / "queries.json").write_text(json.dumps({"ids": [q["id"] for q in queries], "texts": [q["text"] for q in queries]}))
    shutil.copyfile(folder / "qrels.json", out / "qrels.json")
    shutil.copyfile(folder / "input.json", out / "input.json")
    if (out / "embed.json").exists() and (out / "docs.f32").exists() and (out / "queries.f32").exists():
        log(f"{dataset}: reusing the vectors already in {out}")
        (out / "input.sha").write_text(stamp)
        return out
    started = time.time()
    subprocess.run([str(embed_bin), "embed", "--input", str(out / "input.json"), "--doc-vectors", str(out / "docs.f32"),
                    "--query-vectors", str(out / "queries.f32"), "--out", str(out / "embed.json")], check=True)
    log(f"{dataset}: embedded {len(docs)} documents and {len(queries)} queries in {time.time() - started:.0f}s")
    (out / "input.sha").write_text(stamp)
    return out


def node(*args, timeout=7200):
    return common.run_measured(["node", "--max-old-space-size=12288", str(WORKER), *map(str, args)], cwd=JS, timeout=timeout)


def build_store(store, data, work):
    path = work / f"store-{store}"
    if path.exists():
        shutil.rmtree(path)
    result = work / "build.json"
    _, _, wall, peak = node("build", store, data, path, result)
    report = json.loads(result.read_text())
    return {"build_seconds": report["build_seconds"], "ingest_s": report["ingest_s"], "close_s": report["close_s"], "wall_seconds": wall, "peak_rss_bytes": peak, "index_bytes": common.dir_bytes(path)}


def serve_row(row, data, work):
    _, store, _ = ROWS[row]
    result = work / "serve.json"
    limit = [BM25_QUERIES] if row == "vectra-bm25" else []  # a fresh process per round, so the heap never accumulates
    _, _, wall, peak = node("serve", row, data, work / f"store-{store}", result, *limit)
    report = json.loads(result.read_text())
    report["wall_seconds"] = wall
    report["peak_rss_bytes"] = peak
    report["load"] = common.LAST_LOAD
    return report


def quality(ids, qrels):
    ndcg = {q: common.ndcg_at(found, qrels.get(q, {}), K) for q, found in ids}
    recall = {q: common.recall_at(found, qrels.get(q, {}), K) for q, found in ids}
    return ndcg, recall


def run(dataset, embed_bin, out_dir=None, rounds=ROUNDS, build_repeats=BUILD_REPEATS, suite="standard"):
    data = prepare(dataset, embed_bin)
    work = Path(os.environ.get("WARM_BENCH_JS_SCRATCH", WORK / "scratch")) / dataset
    work.mkdir(parents=True, exist_ok=True)
    qrels = json.loads((data / "qrels.json").read_text())
    pinned = textdata.DATASET_DIGESTS.get(DATASETS[dataset])
    digests = {"input_sha256": sha256(data / "input.json"), "qrels_sha256": sha256(data / "qrels.json")}
    if pinned and (digests["input_sha256"], digests["qrels_sha256"]) != pinned:
        raise SystemExit(f"{dataset}: input/qrels differ from the pinned digests of the published text results")
    hw = common.hardware()
    log(f"{dataset}: building stores on {hw['class']}")
    stores = [st for st in STORES if st != "vectra-bm25" or BM25_QUERIES > 0]
    builds = {store: [] for store in stores}
    for repeat in range(build_repeats):
        for store in stores:
            builds[store].append(build_store(store, data, work))
            log(f"  build {store} #{repeat + 1}: {builds[store][-1]['build_seconds']:.2f}s")
    # the last repeat of every store stays on disk for the serve rounds
    active = [row for row in ROWS if row != "vectra-bm25" or BM25_QUERIES > 0]
    systems = {row: {"display": ROWS[row][0], "contract": ROWS[row][2], "store": ROWS[row][1], "serve_rounds": []} for row in active}
    order = list(active)
    for r in range(rounds):
        rotated = order[r % len(order):] + order[:r % len(order)]
        for row in rotated:
            report = serve_row(row, data, work)
            systems[row]["serve_rounds"].append({k: v for k, v in report.items() if k not in ("ids",)})
            if r == 0:
                ndcg, recall = quality(report["ids"], qrels)
                systems[row]["quality"] = {"queries": len(ndcg), "ndcg@10": statistics.fmean(ndcg.values()), "recall@10": statistics.fmean(recall.values()),
                                           "per_query_ndcg@10": {q: round(v, 6) for q, v in ndcg.items()}}
            elif report["ids"] != serve_ids(systems[row]):
                systems[row]["quality"]["unstable_across_rounds"] = True
            if r == 0:
                systems[row]["_ids"] = report["ids"]
            log(f"  round {r + 1} {row}: p50 {report['passes'][0]['p50_ns'] / 1e3:.0f}us")
    for row in systems:
        systems[row].pop("_ids", None)
    for store in stores:
        for row, entry in systems.items():
            if entry["store"] == store:
                entry["build_runs"] = builds[store]
    result = {
        "schema_version": 1, "kind": "js", "suite": suite, "commit": common.commit(), "harness_commit": os.environ.get("WARM_BENCH_HARNESS_COMMIT", common.commit()), "date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S%z"),
        "hardware": hw,
        "dataset": {"id": DATASETS[dataset], "name": dataset, "documents": len(json.loads((data / "corpus.json").read_text())["ids"]),
                    "queries": len(json.loads((data / "queries.json").read_text())["ids"]), **digests},
        "embedding": json.loads((data / "embed.json").read_text()),
        "versions": versions(),
        "protocol": {
            "k": K, "rounds": rounds, "build_repeats": build_repeats, "passes_per_round": 3, "warm_up": "one untimed pass over the queries before the timed passes",
            "retrieval": "exact cosine top-10 over the same 384-dimension document vectors from one file; query vectors supplied (query embedding is outside the "
                         "timer for every system and is shared)",
            "timing": "one query at a time from one Node process; each call awaited and the hit ids read inside the timer. Vectra `LocalIndex.queryItems` and "
                      "warm-memory `warm-memory/vectra` `queryItems` (mode exact) for the dense rows; `searchDocuments` for the WARM hybrid and lexical rows",
            "cold_start": "process start (node and module load included) to the end of the first query",
            "build": "fresh process per repeat; vectors are read from disk before the timer; WARM builds `warm-vector` (vector records) and `warm-docs` (documents "
                     "with their embeddings, text not stored, one chunk per document); Vectra builds `LocalIndex.batchInsertItems`",
            "size": "allocated bytes of the store directory (du)", "peak_rss": "wait4 rusage of the serve or build process",
            "interleaving": "each round runs every row once in a fresh process, order rotated per round",
            "vectra_bm25": f"Vectra's isBm25 rebuilds a BM25 engine over every non-selected item per query (about 100 ms at 300 documents) and runs out of heap after "
                           f"about 20 queries in one process, so each round measures {BM25_QUERIES} queries (set per dataset: its cost grows with the corpus text) in a fresh process, one timed pass, no warm-up. "
                           "Its list is the 10 semantic hits followed by BM25 hits, so nDCG@10 and recall@10 are those of the dense row by construction.",
            "docreader": "Vectra's BM25 path reads each chunk's text through `docReader`; the benchmark supplies it from memory, which favours Vectra (its default reads one file per document per query)",
        },
        "systems": systems,
        "unsupported": UNSUPPORTED,
        "not_run": {} if BM25_QUERIES > 0 else {"vectra-bm25": "Vectra's isBm25 rebuilds a BM25 engine over every item per query; its cost grows with the corpus (about 46 s per query at 4,559 LongMemEval sessions, 6 to 8 s at 3,600 to 5,200 short documents), so at 57,600 documents the row was not run (WARM_BENCH_JS_BM25_QUERIES=0)."},
    }
    out = Path(out_dir) if out_dir else common.RESULTS
    name = f"{suite}-js-{dataset}-{hw['class']}.json"
    common.write_json(out / name, result)
    log(f"wrote {out / name}")
    return out / name


def serve_ids(entry):
    return entry.get("_ids")
