# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Orchestrator of the hybrid contract (README.md).

  hybrid_run.py prepare  --work DIR            inputs, shared document and query vectors for every dataset
  hybrid_run.py dev      --work DIR --out DIR  builds on the development corpora; untimed lists of WARM and every comparator
  hybrid_run.py eval     --work DIR --out DIR --rounds 10 --frozen FILE
                                               builds, interleaved pinned serve rounds, quality and speed results per corpus
"""

import argparse
import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from warmbench import jars, textdata  # noqa: E402
from warmbench.common import dir_bytes, hardware, ndcg_at, recall_at, run_measured, source_record  # noqa: E402

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
EVAL = {"fiqa": "beir:fiqa", "scifact": "beir:scifact", "nfcorpus": "beir:nfcorpus", "memory": "memory", "arguana": "beir:arguana", "scidocs": "beir:scidocs"}
# development sets: (corpus key, dataset id). FiQA and NFCorpus train queries share the evaluation corpora.
DEV = {"fiqa-train": ("fiqa", "beir:fiqa:train"), "nfcorpus-train": ("nfcorpus", "beir:nfcorpus:train"), "memory-selection": ("memory-selection", "memory-selection")}
DEV_QUERY_CAP = 1000
# Corpora above these sizes are subsampled to fit the one-VM time budget: every document judged relevant to any evaluation
# or sampled development query of the corpus is kept, the rest is a seeded random fill. Recorded in each meta.json.
CORPUS_CAP = {"fiqa": 15000, "scidocs": 10000}
DEV_SEED = 20261114
SINGLE = {n: "1" for n in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "RAYON_NUM_THREADS", "NUMEXPR_NUM_THREADS")}
PY = os.environ.get("HYBRID_PY", str(Path.home() / "hvenv/bin/python"))
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", Path.home() / "target"))
BIN = TARGET / "dist/examples/competitive_hybrid"
PIN_CPU = os.environ.get("HYBRID_PIN_CPU", "2")
COMPARATORS = ["lucene-hnsw", "lucene-exact", "sqlite-vec", "lancedb", "tantivy-usearch"]
BUILD_OF = {"warm": "warm", "lucene-hnsw": "lucene", "lucene-exact": "lucene", "sqlite-vec": "sqlite-vec", "lancedb": "lancedb", "tantivy-usearch": "tantivy-usearch"}
FOREIGN_LIMIT = float(os.environ.get("HYBRID_FOREIGN_LIMIT", "0.05"))


def log(message):
    print(time.strftime("%H:%M:%S"), message, file=sys.stderr, flush=True)


def corpus_key(dataset):
    return {"memory": "memory", "memory-selection": "memory-selection"}.get(dataset) or dataset.split(":")[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def busy_seconds():
    """Busy CPU seconds of the pinned core from /proc/stat: user, nice, system, irq, softirq, steal. Everything that runs on
    it during a serve phase is the worker and its helper (whose CPU the worker reports), so the rest is foreign."""
    line = next(l for l in Path("/proc/stat").read_text().splitlines() if l.startswith(f"cpu{PIN_CPU} "))
    fields = line.split()[1:]
    user, nice, system, idle, iowait, irq, softirq, steal = (int(x) for x in fields[:8])
    return (user + nice + system + irq + softirq + steal) / os.sysconf("SC_CLK_TCK")


# ---------------------------------------------------------------- prepare

def subsample(name, documents):
    relevant = set()
    for dataset, is_dev in ((EVAL[name], False), *[(d, True) for c, d in DEV.values() if c == name]):
        folder = textdata.load(dataset)
        qrels = json.loads((folder / "qrels.json").read_text())
        queries = json.loads((folder / "input.json").read_text())["queries"]
        if is_dev and len(queries) > DEV_QUERY_CAP:
            queries = random.Random(DEV_SEED).sample(queries, DEV_QUERY_CAP)
        for q in queries:
            relevant |= {d for d, g in qrels[q["id"]].items() if g > 0}
    keep = [d for d in documents if d["id"] in relevant]
    rest = [d for d in documents if d["id"] not in relevant]
    random.Random(DEV_SEED).shuffle(rest)
    chosen = {d["id"] for d in keep + rest[: max(0, CORPUS_CAP[name] - len(keep))]}
    return [d for d in documents if d["id"] in chosen]


def prepare_dataset(dataset, work, is_dev):
    key = dataset.replace(":", "-")
    folder = textdata.load(dataset)
    out = work / key
    out.mkdir(parents=True, exist_ok=True)
    corpus = work / "corpus" / corpus_key(dataset)
    corpus.mkdir(parents=True, exist_ok=True)
    input_json = json.loads((folder / "input.json").read_text())
    qrels = json.loads((folder / "qrels.json").read_text())
    queries = input_json["queries"]
    if is_dev and len(queries) > DEV_QUERY_CAP:
        queries = random.Random(DEV_SEED).sample(queries, DEV_QUERY_CAP)
    key_name = corpus_key(dataset)
    if key_name in CORPUS_CAP:
        input_json["documents"] = subsample(key_name, input_json["documents"])
    ids_sha = hashlib.sha256("\n".join(d["id"] for d in input_json["documents"]).encode()).hexdigest()
    marker = corpus / "ids.sha"
    if not marker.exists():
        (corpus / "input.json").write_text(json.dumps(input_json))
        (corpus / "corpus.tsv").write_text("".join(f'{d["id"]}\t{" ".join(d["text"].split())}\n' for d in input_json["documents"]))
        log(f"embedding documents of {corpus.name}: {len(input_json['documents'])}")
        t = time.time()
        subprocess.run([str(BIN), "embed", "--input", str(corpus / "input.json"), "--doc-vectors", str(corpus / "docs.f32"), "--out", str(corpus / "embed-docs.json")],
                       check=True, env={**os.environ, "HF_HUB_OFFLINE": "0"})
        marker.write_text(ids_sha)
        log(f"documents embedded in {time.time() - t:.0f}s")
    elif marker.read_text() != ids_sha:
        raise SystemExit(f"{dataset}: document ids differ from the corpus the vectors were embedded for")
    serve_input = {"documents": [], "queries": [{"id": q["id"], "text": q["text"]} for q in queries]}
    (out / "serve-input.json").write_text(json.dumps(serve_input))
    (out / "queries.tsv").write_text("".join(f'{q["id"]}\t{" ".join(q["text"].split())}\n' for q in queries))
    (out / "qrels.json").write_text(json.dumps({q["id"]: qrels[q["id"]] for q in queries}))
    subprocess.run([str(BIN), "embed", "--input", str(out / "serve-input.json"), "--query-vectors", str(out / "queries.f32"), "--embed-docs", "false",
                    "--threads", "4", "--out", str(out / "embed-queries.json")], check=True)
    return out


def prepare(a):
    work = Path(a.work)
    # Smallest corpora first, so every later phase can be exercised before the largest embedding finishes.
    order = ["nfcorpus", "scifact", "memory", "arguana", "scidocs", "memory-selection", "nfcorpus-train", "fiqa", "fiqa-train"]
    datasets = {**{n: (d, False) for n, d in EVAL.items()}, **{n: (d, True) for n, (_, d) in DEV.items()}}
    for name in order:
        dataset, is_dev = datasets[name]
        prepare_dataset(dataset, work, is_dev)
        log(f"prepared {name}")
    log("prepare done")


# ---------------------------------------------------------------- phases

def java_cmd(phase, *args):
    classpath, paths = jars.ensure()
    classes = Path(classpath.split(":")[-1]).parent / "classes-hybrid"
    stamp = classes / f".{sha(HERE / 'LuceneHybrid.java')[:16]}"
    if not stamp.exists():
        shutil.rmtree(classes, ignore_errors=True)
        classes.mkdir()
        subprocess.run(["javac", "--release", "21", "-cp", ":".join(map(str, paths)), "-d", str(classes), str(HERE / "LuceneHybrid.java")], check=True)
        stamp.write_text("")
    flags = ["-Xms512m", "-Xmx2g", "-XX:+UseSerialGC", "-XX:-UsePerfData"] if phase == "serve" else ["-XX:MaxRAMPercentage=25", "-XX:+UseParallelGC", "-XX:-UsePerfData"]
    return ["java", *flags, "-cp", ":".join([*map(str, paths), str(classes)]), "LuceneHybrid", phase, *map(str, args)]


def build_cmd(system, corpus, store, out):
    if system == "warm":
        return [str(BIN), "build", "--input", str(corpus / "input.json"), "--doc-vectors", str(corpus / "docs.f32"), "--store", str(store), "--out", str(out)]
    if system == "lucene":
        return java_cmd("build", corpus / "corpus.tsv", corpus / "docs.f32", store, out)
    return [PY, "-I", str(HERE / "hybrid_worker.py"), "build", "--system", system, "--input", str(corpus / "input.json"),
            "--doc-vectors", str(corpus / "docs.f32"), "--store", str(store), "--out", str(out)]


def serve_cmd(row, dataset_dir, store, out, rrf_ks, lists=False, warm_flags=(), rows=10**9):
    queries = dataset_dir / "queries.tsv"
    vectors = dataset_dir / "queries.f32"
    if row == "warm":
        if lists:
            return [str(BIN), "dump", "--input", str(dataset_dir / "serve-input.json"), "--query-vectors", str(vectors), "--store", str(store), "--out", str(out)]
        return [str(BIN), "serve", "--input", str(dataset_dir / "serve-input.json"), "--query-vectors", str(vectors), "--store", str(store), "--out", str(out), "--including", "50", "--timed", "200", *warm_flags]
    if row.startswith("lucene"):
        return java_cmd("serve", queries, vectors, store, out, "exact" if row == "lucene-exact" else "hnsw", ",".join(map(str, rrf_ks)), 2, 50, 200, BIN, "lists" if lists else "timed", rows)
    return [PY, "-I", str(HERE / "hybrid_worker.py"), "serve", "--system", row, "--input", str(dataset_dir / "serve-input.json"), "--query-vectors", str(vectors),
            "--store", str(store), "--out", str(out), "--embed-bin", str(BIN), "--rrf-ks", ",".join(map(str, rrf_ks)), "--rows", str(rows), "--including", "50", "--timed", "200", *(["--lists"] if lists else [])]


def build_all(corpus_name, work, systems, repeats):
    corpus = work / "corpus" / corpus_name
    builds = {}
    for system in dict.fromkeys(BUILD_OF[s] for s in systems):
        store = work / "stores" / corpus_name / system
        runs = []
        for _ in range(repeats):
            shutil.rmtree(store, ignore_errors=True)
            store.parent.mkdir(parents=True, exist_ok=True)
            out = store.with_suffix(".build.json")
            _, _, wall, peak = run_measured(build_cmd(system, corpus, store, out), env=SINGLE if system in ("warm", "sqlite-vec") else None, timeout=7200)
            runs.append(json.loads(out.read_text()) | {"peak_rss_bytes": peak, "wall_s": wall, "index_bytes": dir_bytes(store)})
            log(f"{corpus_name} {system} build {runs[-1]['ingest_s']:.1f}s {runs[-1]['index_bytes'] / 2**20:.0f} MiB")
        builds[system] = runs
    return builds


def timed_serve(cmd, out, pin=True):
    """One serve phase pinned to a core, repeated while the pinned core's foreign CPU exceeds FOREIGN_LIMIT of a core."""
    for attempt in range(3):
        before, start = busy_seconds(), time.perf_counter()
        _, _, wall, peak = run_measured(["taskset", "-c", PIN_CPU, *cmd] if pin else cmd, env=SINGLE, timeout=7200)
        elapsed = time.perf_counter() - start
        body = json.loads(Path(out).read_text())
        foreign = max(0.0, (busy_seconds() - before - body.get("cpu_s", 0.0)) / elapsed)
        if foreign <= FOREIGN_LIMIT or not pin:
            break
        log(f"foreign CPU {foreign:.3f} cores during {cmd[0]}; repeating")
    return body | {"peak_rss_bytes": peak, "wall_s": wall, "foreign_cpu_cores": foreign, "foreign_attempts": attempt + 1}


# ---------------------------------------------------------------- dev

def dev(a):
    work, out = Path(a.work), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    systems = ["warm", *COMPARATORS]
    stored = {}
    for corpus_name in dict.fromkeys(c for c, _ in DEV.values()):
        stored[corpus_name] = build_all(corpus_name, work, systems, 1)
    for name, (corpus_name, dataset) in DEV.items():
        dataset_dir = work / dataset.replace(":", "-")
        for row in systems:
            store = work / "stores" / corpus_name / BUILD_OF[row]
            target = out / f"{name}-{row}.json"
            if target.exists():
                continue
            tmp = work / "tmp-dev.json"
            run_measured(serve_cmd(row, dataset_dir, store, tmp, [60], lists=True), env=SINGLE, timeout=7200)
            shutil.move(tmp, target)
            log(f"dev lists {name} {row}")
        shutil.copy(dataset_dir / "qrels.json", out / f"{name}-qrels.json")
    (out / "builds.json").write_text(json.dumps(stored, indent=1))


# ---------------------------------------------------------------- eval

def quality(rows, qrels):
    ndcg = [ndcg_at([h[0] for h in r["results"]], qrels[r["id"]], 10) for r in rows]
    recall = [recall_at([h[0] for h in r["results"]], qrels[r["id"]], 100) for r in rows]
    return {"ndcg_at_10": sum(ndcg) / len(ndcg), "recall_at_100": sum(recall) / len(recall), "per_query_ndcg_at_10": ndcg, "per_query_recall_at_100": recall, "queries": len(rows)}


def row_names(frozen):
    """(row name, system, rrf constant or None). Comparators get a default row and, when it differs, a tuned row."""
    rows = [("warm", "warm", None)]
    if frozen.get("warm_previous_flags"):
        rows.append(("warm-previous", "warm", None))
    for system in COMPARATORS:
        rows.append((system, system, 60))
        tuned = frozen["comparator_rrf"][system]
        if tuned != 60:
            rows.append((f"{system}+tuned", system, tuned))
    return rows


def evaluate(a):
    work, out = Path(a.work), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    frozen = json.loads(Path(a.frozen).read_text())
    rows = row_names(frozen)
    source = source_record("hybrid")
    for name in a.corpora.split(","):
        dataset = EVAL[name]
        corpus_name = corpus_key(dataset)
        dataset_dir = work / dataset.replace(":", "-")
        qrels = json.loads((dataset_dir / "qrels.json").read_text())
        builds = build_all(corpus_name, work, [r[1] for r in rows], a.builds)
        result = {"contract": "hybrid", "dataset": dataset, "hardware": hardware(), "commit": source["commit"], "source": source,
                  "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "frozen": frozen, "rounds": a.rounds, "pin_cpu": PIN_CPU,
                  "builds": builds, "systems": {r[0]: {"rounds": []} for r in rows}, "queries": len(qrels)}
        systems = list(dict.fromkeys(r[1] for r in rows))
        for r in range(a.rounds):
            order = systems[r % len(systems):] + systems[:r % len(systems)]
            for system in order:
                store = work / "stores" / corpus_name / BUILD_OF[system]
                mine = [x for x in rows if x[1] == system]
                tmp = work / f"serve-{system}.json"
                if system == "warm":
                    for rowname, _, _ in mine:
                        flags = frozen["warm_previous_flags"] if rowname == "warm-previous" else frozen.get("warm_flags", [])
                        body = timed_serve(serve_cmd("warm", dataset_dir, store, tmp, [], warm_flags=flags), tmp)
                        result["systems"][rowname]["rounds"].append(summarise(body, body, None, qrels if r == 0 else None))
                else:
                    constants = [m[2] for m in mine]
                    body = timed_serve(serve_cmd(system, dataset_dir, store, tmp, constants, rows=10**9 if r == 0 else 50), tmp)
                    for rowname, _, constant in mine:
                        result["systems"][rowname]["rounds"].append(summarise(body, body["configs"][str(float(constant)) if system in ("sqlite-vec", "lancedb", "tantivy-usearch") else str(constant)], constant, qrels if r == 0 else None))
                    result["systems"][mine[0][0]].setdefault("versions", body.get("versions"))
            log(f"{name} round {r + 1}/{a.rounds} done")
        for row, system, _ in rows:
            result["systems"][row]["system"] = system
            result["systems"][row]["index_bytes"] = builds[BUILD_OF[system]][0]["index_bytes"]
        (out / f"hybrid-{name}.json").write_text(json.dumps(result))
        log(f"{name} written")


def summarise(body, config, constant, qrels):
    from warmbench.common import pass_summary
    if "configs" not in body:  # WARM: the report is the config
        config = body
    summary = {"including": pass_summary(config["including_ns"]), "embed": pass_summary(config["embed_ns"]),
               "excluding": [pass_summary(ns) for ns in config["excluding_passes_ns"]],
               "including_id_mismatches": config["including_id_mismatches"], "peak_rss_bytes": body["peak_rss_bytes"],
               "foreign_cpu_cores": body["foreign_cpu_cores"], "foreign_attempts": body["foreign_attempts"], "cpu_s": body.get("cpu_s")}
    if qrels is not None:
        summary["quality"] = quality(config["rows"], qrels)
    return summary


def main():
    p = argparse.ArgumentParser()
    p.add_argument("phase", choices=["prepare", "dev", "eval"])
    p.add_argument("--work", required=True)
    p.add_argument("--out")
    p.add_argument("--rounds", type=int, default=10)
    p.add_argument("--builds", type=int, default=2)
    p.add_argument("--frozen")
    p.add_argument("--corpora", default=",".join(EVAL))
    a = p.parse_args()
    {"prepare": prepare, "dev": dev, "eval": evaluate}[a.phase](a)


if __name__ == "__main__":
    main()
