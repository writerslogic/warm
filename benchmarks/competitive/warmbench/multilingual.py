# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Multilingual lexical quality: WARM BM25 (one or more pinned binaries) against Lucene analyzers
on MIRACL dev, nDCG@10 per language. Quality only; nothing here is timed.

Corpus per language: every judged passage of the dev qrels plus distractors drawn uniformly
(fixed seed) from the rest of the MIRACL corpus up to `--corpus` passages, identical for every
system. Absolute scores are therefore higher than full-corpus MIRACL figures; the comparison
between systems on one corpus is what the file reports.

  uv run --no-project python -I benchmarks/competitive/warmbench/multilingual.py \\
      --raw DIR --langs hi fa th sw --warm before=BIN after=BIN \\
      --lucene standard hindi persian thai --out benchmarks/results/FILE.json

DIR holds topics-<l>.tsv, qrels-<l>.tsv (miracl/miracl dev) and corpus-<l>-docs-*.jsonl.gz
(miracl/miracl-corpus v1.0).
"""

import argparse
import gzip
import hashlib
import json
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from warmbench import jars  # noqa: E402
from warmbench.common import log, ndcg_at, recall_at  # noqa: E402

# Lucene analyzers that match each language; `standard` (UAX#29 + lowercase) runs on all.
LANGUAGE_ANALYZER = {"hi": "hindi", "fa": "persian", "th": "thai", "ar": "arabic", "fi": "finnish",
                     "id": "indonesian", "tr": "turkish"}


def build_input(raw, lang, corpus_size, seed, work):
    folder = work / f"miracl-{lang}-dev-{corpus_size}"
    if (folder / "input.json").exists():
        return folder
    qrels = {}
    for line in (raw / f"qrels-{lang}.tsv").open(encoding="utf-8"):
        qid, _, did, grade = line.rstrip("\n").split("\t")
        qrels.setdefault(qid, {})[did] = int(grade)
    judged = {did for row in qrels.values() for did in row}
    passages, others = {}, []
    shards = sorted(raw.glob(f"corpus-{lang}-docs-*.jsonl.gz"))
    for shard in shards:
        with gzip.open(shard, "rt", encoding="utf-8") as stream:
            for line in stream:
                row = json.loads(line)
                text = (row.get("title", "") + "\n" + row["text"]).strip()
                if row["docid"] in judged:
                    passages[row["docid"]] = text
                else:
                    others.append((row["docid"], text))
    missing = len(judged - passages.keys())
    rng = random.Random(seed)
    take = max(0, corpus_size - len(passages))
    total_others = len(others)
    for did, text in rng.sample(others, min(take, len(others))):
        passages[did] = text
    del others
    documents = [{"id": did, "text": text, "metadata": {"bucket": 0}, "chunkWords": 4096,
                  "overlapWords": 0, "storeText": False}
                 for did, text in sorted(passages.items()) if text]
    options = {"k": 100, "candidateLimit": 100, "lexicalWeight": 1, "semanticWeight": 0}
    queries = []
    for line in (raw / f"topics-{lang}.tsv").open(encoding="utf-8"):
        qid, text = line.rstrip("\n").split("\t", 1)
        if any(g > 0 for g in qrels.get(qid, {}).values()):
            queries.append({"id": qid, "text": text, "options": options})
    folder.mkdir(parents=True, exist_ok=True)
    body = json.dumps({"documents": documents, "queries": queries}, ensure_ascii=False) + "\n"
    (folder / "input.json").write_text(body, encoding="utf-8")
    (folder / "qrels.json").write_text(json.dumps(qrels) + "\n")
    meta = {"dataset": f"miracl-v1.0-{lang}", "split": "dev", "documents": len(documents),
            "judged_passages": len(judged), "judged_missing_from_corpus": missing,
            "corpus_passages_available": total_others + len(judged) - missing,
            "distractor_seed": seed, "queries": len(queries),
            "input_sha256": hashlib.sha256(body.encode()).hexdigest(),
            "shards": [s.name for s in shards]}
    (folder / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    return folder


def run(cmd):
    done = subprocess.run(cmd, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed ({done.returncode}): {done.stderr[-2000:]}")


def rows_of(system_cmd, folder, store, out):
    shutil.rmtree(store, ignore_errors=True)
    t0 = str(time.time_ns())
    run(system_cmd("build", folder / "input.json", store, out, t0))
    run(system_cmd("serve", folder / "input.json", store, out, t0))
    rows = json.loads(out.read_text())["rows"]
    shutil.rmtree(store, ignore_errors=True)
    return rows


def quality(rows, qrels):
    ndcg = [ndcg_at([h["documentId"] for h in r["results"]], qrels[r["id"]], 10) for r in rows]
    rec = [recall_at([h["documentId"] for h in r["results"]], qrels[r["id"]], 100) for r in rows]
    return {"ndcg_at_10": sum(ndcg) / len(ndcg), "recall_at_100": sum(rec) / len(rec), "queries": len(rows),
            "per_query_ndcg_at_10": ndcg}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--raw", type=Path, required=True)
    p.add_argument("--work", type=Path, default=Path.home() / ".cache/warm-bench/competitive/multilingual")
    p.add_argument("--langs", nargs="+", required=True)
    p.add_argument("--corpus", type=int, default=100_000)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--warm", nargs="+", default=[], metavar="LABEL=BINARY")
    p.add_argument("--lucene", nargs="*", default=["standard"], help="standard and/or language analyzers")
    p.add_argument("--extra", nargs="*", default=[], metavar="NAME=FOLDER",
                   help="also run an existing warm-bench text folder (input.json, qrels.json)")
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args()
    a.work.mkdir(parents=True, exist_ok=True)
    classpath, _ = jars.ensure(jars.VERSION)
    results = json.loads(a.out.read_text()) if a.out.exists() else {"languages": {}}
    results["protocol"] = {
        "dataset": "MIRACL v1.0 dev (miracl/miracl topics+qrels, miracl/miracl-corpus passages)",
        "corpus": f"all judged passages + uniform fixed-seed distractors up to {a.corpus} passages per language",
        "queries": "dev topics with at least one positive judgment",
        "retrieval": "lexical only, top 100; WARM `--encode lexical`; Lucene BM25Similarity defaults, "
                     "whole query text through the analyzer",
        "lucene": jars.VERSION, "metric": "nDCG@10 over graded qrels (MIRACL grades 0/1)",
    }
    sets = [(lang, None) for lang in a.langs] + [tuple(e.split("=", 1)) for e in a.extra]
    for lang, given in sets:
        folder = Path(given) if given else build_input(a.raw, lang, a.corpus, a.seed, a.work)
        qrels = json.loads((folder / "qrels.json").read_text())
        entry = results["languages"].setdefault(lang, {})
        entry["meta"] = json.loads((folder / "meta.json").read_text())
        systems = entry.setdefault("systems", {})
        rankings = {}
        for spec in a.warm:
            label, binary = spec.split("=", 1)
            cmd = lambda phase, i, s, o, t0, b=binary: [b, phase, "--input", str(i), "--store", str(s), "--out", str(o),
                                                        "--t0-ns", t0, "--encode", "lexical", "--passes", "1"]
            rows = rows_of(cmd, folder, a.work / "store-warm", a.work / f"{lang}-warm-{label}.json")
            q = quality(rows, qrels)
            q["binary_sha256"] = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
            systems[f"warm-{label}"] = q
            rankings[label] = {r["id"]: [(h["documentId"], h["lexicalScore"]) for h in r["results"]] for r in rows}
        if len(rankings) > 1:
            first, *rest = rankings
            entry["warm_identity"] = {
                f"{first}-vs-{other}": {
                    "queries": len(rankings[first]),
                    "identical_top100_ids": sum([d for d, _ in rankings[first][k]] == [d for d, _ in v]
                                                for k, v in rankings[other].items()),
                    "identical_top100_ids_and_scores": sum(rankings[first][k] == v for k, v in rankings[other].items()),
                } for other in rest}
            log(f"{lang} warm-{label} {q['ndcg_at_10']:.4f}")
        for analyzer in a.lucene:
            if analyzer == "language":
                analyzer = LANGUAGE_ANALYZER.get(lang)
                if analyzer is None:
                    continue
            cmd = lambda phase, i, s, o, t0, an=analyzer: ["java", "-cp", classpath, "LuceneBm25", phase, str(i), str(s),
                                                           str(o), t0, "1", "100", an]
            q = quality(rows_of(cmd, folder, a.work / "store-lucene", a.work / f"{lang}-lucene-{analyzer}.json"), qrels)
            systems[f"lucene-{analyzer}"] = q
            log(f"{lang} lucene-{analyzer} {q['ndcg_at_10']:.4f}")
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps(results, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
