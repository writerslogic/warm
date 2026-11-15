# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Python comparators of the hybrid contract (README.md): SQLite FTS5 + sqlite-vec, LanceDB hybrid search, and
tantivy + usearch with application-side RRF. Phases `build` and `serve`; the report shape matches
examples/competitive_hybrid.rs. Query vectors for the timed from-text pass come from the shared
`competitive_hybrid embed-serve` helper, so every comparator embeds with WARM's embedder."""

import argparse
import itertools
import json
import os
import resource
import sqlite3
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

K = 100
DIM = 384


def words(text):
    return sorted({"".join(chars) for kind, chars in itertools.groupby(text.lower(), str.isalnum) if kind})


def rrf(lists, constant, k=K):
    """Application-side reciprocal rank fusion of ranked id lists: sum of 1 / (constant + rank), rank from 1."""
    score = {}
    for ranked in lists:
        for rank, key in enumerate(ranked, 1):
            score[key] = score.get(key, 0.0) + 1.0 / (constant + rank)
    return sorted(score.items(), key=lambda kv: -kv[1])[:k]


def own_cpu():
    ru = resource.getrusage(resource.RUSAGE_SELF)
    return ru.ru_utime + ru.ru_stime


def peak_kib():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss


class Helper:
    """The shared query-embedding helper process (same core as this worker: it inherits the affinity)."""

    def __init__(self, binary):
        self.proc = subprocess.Popen([binary, "embed-serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)

    def embed(self, text):
        raw = text.encode("utf-8")
        self.proc.stdin.write(struct.pack("<I", len(raw)) + raw)
        return np.frombuffer(self._exact(DIM * 4), dtype="<f4")

    def _exact(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.proc.stdout.read(n - len(buf))
            if not chunk:
                raise RuntimeError("embed-serve helper closed its pipe")
            buf += chunk
        return buf

    def close(self):
        self.proc.stdin.write(struct.pack("<I", 0xFFFFFFFF))
        cpu = struct.unpack("<d", self._exact(8))[0]
        self.proc.wait()
        return cpu


def read_vectors(path, count):
    return np.fromfile(path, dtype="<f4").reshape(count, DIM)


class SqliteVec:
    def __init__(self, store):
        self.store = Path(store)

    def _connect(self):
        import sqlite_vec
        db = sqlite3.connect(self.store / "hybrid.db")
        db.enable_load_extension(True)
        sqlite_vec.load(db)
        db.enable_load_extension(False)
        return db

    def build(self, documents, vectors):
        self.store.mkdir(parents=True)
        db = self._connect()
        db.execute("CREATE VIRTUAL TABLE fts USING fts5(body, content='', contentless_delete=1, tokenize='porter unicode61')")
        db.execute(f"CREATE VIRTUAL TABLE vec USING vec0(embedding float[{DIM}] distance_metric=cosine)")
        db.execute("CREATE TABLE identities (rowid INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL)")
        with db:
            for i, (d, v) in enumerate(zip(documents, vectors), 1):
                db.execute("INSERT INTO fts(rowid, body) VALUES (?, ?)", (i, d["text"]))
                db.execute("INSERT INTO vec(rowid, embedding) VALUES (?, ?)", (i, v.tobytes()))
                db.execute("INSERT INTO identities(rowid, id) VALUES (?, ?)", (i, d["id"]))
        db.execute("INSERT INTO fts(fts) VALUES ('optimize')")
        db.commit()
        db.close()

    def open(self):
        self.db = self._connect()
        self.db.execute("PRAGMA query_only = ON")
        self.db.execute("PRAGMA mmap_size = 1073741824")
        self.db.execute("PRAGMA cache_size = -65536")
        self.db.execute("PRAGMA temp_store = MEMORY")
        self.ids = dict(self.db.execute("SELECT rowid, id FROM identities"))

    @staticmethod
    def expression(text):
        w = words(text)
        return " OR ".join('"' + x + '"' for x in w) if w else '"zzzznomatchzzzz"'

    def lexical(self, text, k=K):
        rows = self.db.execute("SELECT rowid, rank FROM fts WHERE fts MATCH ? ORDER BY rank LIMIT ?", (self.expression(text), k))
        return [(self.ids[r], -s) for r, s in rows]

    def vector(self, vec, k=K):
        rows = self.db.execute("SELECT rowid, distance FROM vec WHERE embedding MATCH ? AND k = ? ORDER BY distance", (vec.tobytes(), k))
        return [(self.ids[r], 1.0 - d) for r, d in rows]

    # Reciprocal rank fusion in one statement, the form of the sqlite-vec documentation's FTS5 + vec0 hybrid example.
    HYBRID = """
    WITH fts_matches AS (
      SELECT rowid, row_number() OVER (ORDER BY rank) AS rank_number FROM fts WHERE fts MATCH :q ORDER BY rank LIMIT :k),
    vec_matches AS (
      SELECT rowid, row_number() OVER (ORDER BY distance) AS rank_number FROM vec WHERE embedding MATCH :v AND k = :k ORDER BY distance),
    final AS (
      SELECT coalesce(f.rowid, v.rowid) AS rowid,
             coalesce(1.0 / (:rrf + f.rank_number), 0.0) + coalesce(1.0 / (:rrf + v.rank_number), 0.0) AS score
      FROM fts_matches f FULL OUTER JOIN vec_matches v ON v.rowid = f.rowid)
    SELECT rowid, score FROM final ORDER BY score DESC LIMIT :k"""

    def hybrid(self, text, vec, constant):
        rows = self.db.execute(self.HYBRID, {"q": self.expression(text), "v": vec.tobytes(), "k": K, "rrf": constant})
        return [(self.ids[r], s) for r, s in rows]

    def versions(self):
        import sqlite_vec
        return {"sqlite": sqlite3.sqlite_version, "sqlite-vec": sqlite_vec.__version__ if hasattr(sqlite_vec, "__version__") else "0.1.9"}


class Lance:
    def __init__(self, store):
        self.store = Path(store)

    def build(self, documents, vectors):
        import lancedb
        import pyarrow as pa
        self.store.mkdir(parents=True)
        db = lancedb.connect(str(self.store))
        table = pa.table({"id": [d["id"] for d in documents], "text": [d["text"] for d in documents],
                          "vector": pa.FixedSizeListArray.from_arrays(pa.array(vectors.ravel(), type=pa.float32()), DIM)})
        tbl = db.create_table("docs", table)
        try:
            tbl.create_fts_index("text", use_tantivy=False, language="English", stem=True, remove_stop_words=True, replace=True)
        except TypeError:
            tbl.create_fts_index("text", use_tantivy=False, replace=True)

    def open(self):
        import lancedb
        from lancedb.rerankers import RRFReranker
        self.reranker = RRFReranker
        self.tbl = lancedb.connect(str(self.store)).open_table("docs")
        self.cache = {}

    def _ranker(self, constant):
        if constant not in self.cache:
            self.cache[constant] = self.reranker(K=constant)
        return self.cache[constant]

    def lexical(self, text, k=K):
        rows = self.tbl.search(" ".join(words(text)) or "zzzznomatchzzzz", query_type="fts").select(["id"]).limit(k).to_list()
        return [(r["id"], r.get("_score", 0.0)) for r in rows]

    def vector(self, vec, k=K):
        rows = self.tbl.search(vec).select(["id"]).limit(k).to_list()
        return [(r["id"], -r.get("_distance", 0.0)) for r in rows]

    def hybrid(self, text, vec, constant):
        rows = (self.tbl.search(query_type="hybrid").vector(vec).text(" ".join(words(text)) or "zzzznomatchzzzz")
                .rerank(self._ranker(constant)).select(["id"]).limit(K).to_list())
        return [(r["id"], r.get("_relevance_score", 0.0)) for r in rows]

    def versions(self):
        import lancedb
        import pyarrow
        return {"lancedb": lancedb.__version__, "pyarrow": pyarrow.__version__}


class TantivyUsearch:
    def __init__(self, store):
        self.store = Path(store)

    def _schema(self):
        import tantivy
        sb = tantivy.SchemaBuilder()
        sb.add_text_field("id", stored=True, tokenizer_name="raw")
        sb.add_text_field("body", stored=False, tokenizer_name="en_stem")
        return sb.build()

    def build(self, documents, vectors):
        import tantivy
        from usearch.index import Index
        self.store.mkdir(parents=True)
        (self.store / "text").mkdir()
        writer = tantivy.Index(self._schema(), path=str(self.store / "text")).writer(heap_size=200_000_000, num_threads=1)
        for d in documents:
            writer.add_document(tantivy.Document(id=d["id"], body=d["text"]))
        writer.commit()
        writer.wait_merging_threads()
        index = Index(ndim=DIM, metric="cos", dtype="f32", connectivity=16, expansion_add=128, expansion_search=128)
        index.add(np.arange(len(documents), dtype=np.uint64), vectors)
        index.save(str(self.store / "vectors.usearch"))
        (self.store / "ids.json").write_text(json.dumps([d["id"] for d in documents]))

    def open(self):
        import tantivy
        from usearch.index import Index
        self.index = tantivy.Index(self._schema(), path=str(self.store / "text"))
        self.index.reload()
        self.searcher = self.index.searcher()
        self.vectors = Index(ndim=DIM, metric="cos", dtype="f32", connectivity=16, expansion_add=128, expansion_search=128)
        self.vectors.load(str(self.store / "vectors.usearch"))
        self.ids = json.loads((self.store / "ids.json").read_text())

    def lexical(self, text, k=K):
        query, _ = self.index.parse_query_lenient(" ".join(words(text)), ["body"])
        return [(self.searcher.doc(addr)["id"][0], score) for score, addr in self.searcher.search(query, k).hits]

    def vector(self, vec, k=K):
        m = self.vectors.search(vec, k)
        return [(self.ids[int(key)], 1.0 - float(d)) for key, d in zip(m.keys, m.distances)]

    def hybrid(self, text, vec, constant):
        return rrf([[i for i, _ in self.lexical(text)], [i for i, _ in self.vector(vec)]], constant)

    def versions(self):
        import tantivy
        import usearch
        return {"tantivy-py": getattr(tantivy, "__version__", "0.27.0"), "usearch": usearch.__version__}


SYSTEMS = {"sqlite-vec": SqliteVec, "lancedb": Lance, "tantivy-usearch": TantivyUsearch}


def load_input(path):
    return json.loads(Path(path).read_text())


def build(a):
    data = load_input(a.input)
    vectors = read_vectors(a.doc_vectors, len(data["documents"]))
    system = SYSTEMS[a.system](a.store)
    start = time.perf_counter()
    system.build(data["documents"], vectors)
    Path(a.out).write_text(json.dumps({"documents": len(data["documents"]), "ingest_s": time.perf_counter() - start, "cpu_s": own_cpu()}))


def serve(a):
    queries = load_input(a.input)["queries"]
    vectors = read_vectors(a.query_vectors, len(queries))
    start = time.perf_counter()
    system = SYSTEMS[a.system](a.store)
    system.open()
    open_s = time.perf_counter() - start
    constants = [float(x) for x in a.rrf_ks.split(",")]
    report = {"open_s": open_s, "configs": {}, "versions": system.versions()}
    if a.lists:
        report["lists"] = [{"id": q["id"], "lex": system.lexical(q["text"]), "vec": system.vector(vectors[i])} for i, q in enumerate(queries)]
        report["cpu_s"] = own_cpu()
        Path(a.out).write_text(json.dumps(report))
        return
    helper = Helper(a.embed_bin)
    timed = min(a.timed, len(queries))
    including = min(a.including, len(queries))
    clock = time.perf_counter_ns
    for constant in constants:
        for i in range(min(16, len(queries))):
            system.hybrid(queries[i]["text"], vectors[i], constant)
        rows = [{"id": q["id"], "results": system.hybrid(q["text"], vectors[i], constant)} for i, q in enumerate(queries[:a.rows])]
        including_ns, embed_ns, mismatched = [], [], 0
        for i in range(including):
            t0 = clock()
            v = helper.embed(queries[i]["text"])
            t1 = clock()
            hits = system.hybrid(queries[i]["text"], v, constant)
            t2 = clock()
            including_ns.append(t2 - t0)
            embed_ns.append(t1 - t0)
            if [h[0] for h in hits] != [h[0] for h in rows[i]["results"]]:
                mismatched += 1
        passes = []
        for _ in range(a.passes):
            ns = []
            for i in range(timed):
                t0 = clock()
                hits = system.hybrid(queries[i]["text"], vectors[i], constant)
                ns.append(clock() - t0)
            passes.append(ns)
        report["configs"][str(constant)] = {"rows": rows, "including_ns": including_ns, "embed_ns": embed_ns,
                                            "excluding_passes_ns": passes, "including_id_mismatches": mismatched}
    helper_cpu = helper.close()
    report["cpu_s"] = own_cpu() + helper_cpu
    report["memory"] = {"vm_hwm_kib": peak_kib()}
    Path(a.out).write_text(json.dumps(report))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("phase", choices=["build", "serve"])
    p.add_argument("--system", required=True, choices=sorted(SYSTEMS))
    p.add_argument("--input", required=True)
    p.add_argument("--store", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--doc-vectors")
    p.add_argument("--query-vectors")
    p.add_argument("--embed-bin", default=os.environ.get("HYBRID_EMBED_BIN"))
    p.add_argument("--rrf-ks", default="60")
    p.add_argument("--passes", type=int, default=2)
    p.add_argument("--rows", type=int, default=10**9)
    p.add_argument("--including", type=int, default=100)
    p.add_argument("--timed", type=int, default=400)
    p.add_argument("--lists", action="store_true")
    a = p.parse_args()
    {"build": build, "serve": serve}[a.phase](a)


if __name__ == "__main__":
    main()
