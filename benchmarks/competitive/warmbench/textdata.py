# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Text datasets as the `{documents, queries}` input JSON plus qrels, identical for every system.

beir:<name>   BEIR train/dev qrels (the existing document-competition protocol: SciFact train,
              NFCorpus dev, FiQA dev; the test qrels stay unread). One chunk per document.
beir:<name>:<split>  any other split, development data for choosing settings (quality_runs.py).
msmarco:<size> MS MARCO passage ranking (BEIR packaging), dev qrels. A fixed seeded sample of QUERY_SAMPLE dev
              queries; the corpus is every passage those queries judge relevant plus a seeded fill, so 100k,
              1m and full are nested. size is 100k, 1m or full (8,841,823 passages).
memory        Agent-memory-shaped: every session of the 100 LongMemEval_S dev questions as one
              searchable memory (answer sessions are the relevant documents); the 400 held-out
              questions are never read, except by `memory-selection`.
memory-selection  100 of those 400 held-out questions (seed 20261114), development data for choosing
              settings; the remaining 300 stay unread.
"""

import json
import random
import urllib.request
import zipfile

from .common import CACHE, ROOT

DATA = CACHE / "competitive" / "data"
BEIR_URL = "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{name}.zip"
LME_URL = ("https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/"
           "98d7416c24c778c2fee6e6f3006e7a073259d48f/longmemeval_s_cleaned.json")
BEIR_SPLITS = {"scifact": "train", "nfcorpus": "dev", "fiqa": "dev"}
MAX_WORDS = 4000
MSMARCO_SEED = 20261114
QUERY_SAMPLE = 1000
MSMARCO_SIZES = {"100k": 100_000, "1m": 1_000_000, "full": None}
SELECTION_SEED = 20261114
K = 100


def _fetch(url, dest):
    if not dest.exists():
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_suffix(dest.suffix + ".part")
        request = urllib.request.Request(url, headers={"User-Agent": "warm-bench/1.0"})
        with urllib.request.urlopen(request) as response, tmp.open("wb") as out:
            while block := response.read(1 << 20):
                out.write(block)
        tmp.rename(dest)
    return dest


def _options():
    return {"k": K, "candidateLimit": K, "lexicalWeight": 1, "semanticWeight": 0}


def _document(identifier, text, bucket):
    return {"id": identifier, "text": text, "metadata": {"bucket": bucket},
            "chunkWords": 4096, "overlapWords": 0, "storeText": False}


def beir(name, split=None):
    """`split` None is the reported evaluation split; any other split is development data for selecting settings."""
    split = split or BEIR_SPLITS[name]
    folder = DATA / f"text-{name}-{split}"
    if (folder / "input.json").exists():
        return folder
    source = CACHE / "downloads" / name
    if not (source / "corpus.jsonl").exists():
        archive = _fetch(BEIR_URL.format(name=name), CACHE / "downloads" / f"{name}.zip")
        with zipfile.ZipFile(archive) as z:
            z.extractall(CACHE / "downloads")
    qrels = {}
    with (source / "qrels" / f"{split}.tsv").open() as stream:
        next(stream)
        for line in stream:
            qid, did, grade = line.rstrip().split("\t")
            qrels.setdefault(qid, {})[did] = int(grade)
    documents = []
    # FiQA has whitespace-only documents; no lexical system can match them, so every system
    # gets the corpus without them and meta.json records how many were left out.
    empty = 0
    for line in (source / "corpus.jsonl").open():
        row = json.loads(line)
        text = (row.get("title", "") + "\n" + row["text"]).strip()
        if not text:
            empty += 1
            continue
        if len(text.split()) > 4096:
            raise ValueError("one complete chunk per document is required")
        documents.append(_document(row["_id"], text, len(row["_id"]) % 17))
    queries = [{"id": q["_id"], "text": q["text"], "options": _options()}
               for q in map(json.loads, (source / "queries.jsonl").open()) if q["_id"] in qrels]
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "input.json").write_text(json.dumps({"documents": documents, "queries": queries}) + "\n")
    (folder / "qrels.json").write_text(json.dumps(qrels) + "\n")
    (folder / "meta.json").write_text(json.dumps({"dataset": name, "split": split, "documents": len(documents),
                                                  "empty_documents_skipped": empty, "queries": len(queries),
                                                  "source": BEIR_URL.format(name=name)}) + "\n")
    return folder


def msmarco(size):
    """Nested subsets of MS MARCO passage, written one document per line so loaders may stream a file
    larger than 2 GiB: `{"documents":[`, one document per line, `],`, then the queries."""
    target = MSMARCO_SIZES[size]
    folder = DATA / f"text-msmarco-{size}-dev"
    if (folder / "input.json").exists():
        return folder
    source = CACHE / "downloads" / "msmarco"
    if not (source / "corpus.jsonl").exists():
        archive = _fetch(BEIR_URL.format(name="msmarco"), CACHE / "downloads" / "msmarco.zip")
        with zipfile.ZipFile(archive) as z:
            z.extractall(CACHE / "downloads")
    qrels_all = {}
    with (source / "qrels" / "dev.tsv").open() as stream:
        next(stream)
        for line in stream:
            qid, did, grade = line.rstrip().split("\t")
            qrels_all.setdefault(qid, {})[did] = int(grade)
    texts = {}
    for line in (source / "queries.jsonl").open():
        row = json.loads(line)
        if row["_id"] in qrels_all:
            texts[row["_id"]] = row["text"]
    chosen = sorted(random.Random(MSMARCO_SEED).sample(sorted(texts), QUERY_SAMPLE))
    qrels = {q: qrels_all[q] for q in chosen}
    relevant = {d for judged in qrels.values() for d in judged}
    ids = [json.loads(line)["_id"] for line in (source / "corpus.jsonl").open()]
    if target is None:
        keep = set(ids)
    else:
        others = [d for d in ids if d not in relevant]
        random.Random(MSMARCO_SEED + 1).shuffle(others)
        keep = relevant | set(others[: max(0, target - len(relevant))])
    folder.mkdir(parents=True, exist_ok=True)
    count = 0
    with (folder / "input.json").open("w") as out:
        out.write('{"documents":[\n')
        for line in (source / "corpus.jsonl").open():
            row = json.loads(line)
            if row["_id"] not in keep:
                continue
            text = (row.get("title", "") + "\n" + row["text"]).strip()
            if not text:
                continue
            out.write(("," if count else "") + json.dumps(_document(row["_id"], text, len(row["_id"]) % 17)) + "\n")
            count += 1
        queries = [{"id": q, "text": texts[q], "options": _options()} for q in chosen]
        out.write('],\n"queries":' + json.dumps(queries) + "}\n")
    (folder / "queries.json").write_text(json.dumps(queries) + "\n")
    (folder / "qrels.json").write_text(json.dumps(qrels) + "\n")
    (folder / "meta.json").write_text(json.dumps({
        "dataset": f"msmarco-passage-dev-{size}", "split": "dev", "documents": count, "queries": len(queries),
        "query_sample": QUERY_SAMPLE, "sample_seed": MSMARCO_SEED, "fill": "relevant passages of the sampled queries plus a seeded shuffle of the rest",
        "corpus_passages": len(ids), "source": BEIR_URL.format(name="msmarco")}) + "\n")
    return folder


def memory(selection=False):
    """The reported 100-question dev set, or with `selection` a seeded sample of 100 questions of the other 400, so
    that settings are chosen on questions the reported set does not contain."""
    folder = DATA / ("text-memory-lme-selection" if selection else "text-memory-lme-dev")
    if (folder / "input.json").exists():
        return folder
    raw = _fetch(LME_URL, CACHE / "downloads" / "longmemeval_s_cleaned.json")
    split = json.loads((ROOT / "benchmarks/public/longmemeval_split.json").read_text())
    dev = set(random.Random(SELECTION_SEED).sample(sorted(split["heldout"]), 100)) if selection else set(split["dev"])
    sessions, queries, qrels = {}, [], {}
    for entry in json.loads(raw.read_text()):
        if entry["question_id"] not in dev:
            continue
        answers = {s: 1 for s in entry["answer_session_ids"]}
        if not answers:
            continue
        for sid, turns in zip(entry["haystack_session_ids"], entry["haystack_sessions"]):
            if sid not in sessions:
                text = "\n".join(f'{t["role"]}: {t["content"]}' for t in turns)
                sessions[sid] = " ".join(text.split()[:MAX_WORDS])
        queries.append({"id": entry["question_id"], "text": entry["question"], "options": _options()})
        qrels[entry["question_id"]] = answers
    documents = [_document(sid, text, len(sid) % 17) for sid, text in sessions.items()]
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "input.json").write_text(json.dumps({"documents": documents, "queries": queries}) + "\n")
    (folder / "qrels.json").write_text(json.dumps(qrels) + "\n")
    (folder / "meta.json").write_text(json.dumps({"dataset": "longmemeval-s-selection-sessions" if selection else "longmemeval-s-dev-sessions", "documents": len(documents),
                                                  "queries": len(queries), "truncated_words": MAX_WORDS, "source": LME_URL}) + "\n")
    return folder


def load(dataset):
    if dataset.startswith("beir:"):
        name, _, split = dataset.split(":", 1)[1].partition(":")
        return beir(name, split or None)
    if dataset.startswith("msmarco:"):
        return msmarco(dataset.split(":", 1)[1])
    if dataset == "memory":
        return memory()
    if dataset == "memory-selection":
        return memory(selection=True)
    raise ValueError(f"unknown text dataset {dataset}")
