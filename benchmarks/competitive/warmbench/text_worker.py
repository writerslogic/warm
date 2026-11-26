# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Lexical comparators run inside their pinned environment. Same phases and output shape as
examples/competitive_text.rs (build, serve, ops) over the {documents, queries} input JSON.

Queries are reduced to the lowercase alphanumeric word set (as the SQLite control in
examples/document_competition.rs does) and combined with OR; each system then applies
its own documented analyzer (stemmer, stop words) and default BM25 parameters.
"""

import argparse
import itertools
import json
import sqlite3
import sys
import time
import zlib
from pathlib import Path

T_IMPORT = time.time_ns()

K = 100


class Unsupported(Exception):
    pass


def words(text):
    return sorted({"".join(chars) for kind, chars in itertools.groupby(text.lower(), str.isalnum) if kind})


def peak_rss():
    import resource
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return rss if sys.platform == "darwin" else rss * 1024


class SqliteFts5:
    def __init__(self, store):
        self.store = Path(store)

    def build(self, documents):
        self.store.mkdir(parents=True)
        db = sqlite3.connect(self.store / "fts.db")
        try:
            db.execute("CREATE VIRTUAL TABLE docs USING fts5(body, content='', contentless_delete=1, tokenize='porter unicode61')")
        except sqlite3.OperationalError:
            # SQLite older than 3.43 cannot delete from a contentless table; fall back to a stored-content
            # table (larger on disk; the SQLite version is recorded in the results).
            db.execute("CREATE VIRTUAL TABLE docs USING fts5(body, tokenize='porter unicode61')")
        db.execute("CREATE TABLE identities (rowid INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL)")
        with db:
            for i, d in enumerate(documents, 1):
                db.execute("INSERT INTO docs(rowid, body) VALUES (?, ?)", (i, d["text"]))
                db.execute("INSERT INTO identities(rowid, id) VALUES (?, ?)", (i, d["id"]))
        db.execute("INSERT INTO docs(docs) VALUES ('optimize')")
        db.commit()
        db.close()

    def open(self):
        self.db = sqlite3.connect(self.store / "fts.db")
        # macOS fsync leaves writes in the drive cache. Every durable replace/delete of WARM ends in
        # fcntl(F_FULLFSYNC) on macOS, as do tantivy (Rust std sync_all/sync_data) and Lucene
        # (JDK FileChannel.force); fullfsync holds SQLite to the same standard.
        self.db.execute("PRAGMA fullfsync = ON")
        self.db.execute("PRAGMA checkpoint_fullfsync = ON")
        # rowid-to-id map held in Python, so the query is a bare MATCH with no join.
        self.ids = dict(self.db.execute("SELECT rowid, id FROM identities"))

    def open_serving(self):
        """Documented read-side settings (sqlite.org pragma and FTS5 docs): a read-only connection, memory-mapped
        reads and a larger page cache."""
        self.open()
        self.db.execute("PRAGMA query_only = ON")
        self.db.execute("PRAGMA mmap_size = 1073741824")
        self.db.execute("PRAGMA cache_size = -65536")
        self.db.execute("PRAGMA temp_store = MEMORY")

    def prepare(self, text):
        return " OR ".join('"' + w + '"' for w in words(text))

    def search(self, expression):
        if not expression:
            return []
        # The documented top-k form: ORDER BY rank (FTS5's hidden bm25 column) with LIMIT, no join and no
        # tie-break the other arms do not pay; ids come from the table held in memory.
        ids = self.ids
        return [(ids[rowid], -score) for rowid, score in self.db.execute(
            "SELECT rowid, rank FROM docs WHERE docs MATCH ? ORDER BY rank LIMIT ?", (expression, K))]

    def replace(self, document):
        row = self.db.execute("SELECT rowid FROM identities WHERE id = ?", (document["id"],)).fetchone()[0]
        with self.db:
            self.db.execute("DELETE FROM docs WHERE rowid = ?", (row,))
            self.db.execute("INSERT INTO docs(rowid, body) VALUES (?, ?)", (row, document["text"] + " benchmarkrevision"))

    def delete(self, document):
        row = self.db.execute("SELECT rowid FROM identities WHERE id = ?", (document["id"],)).fetchone()[0]
        with self.db:
            self.db.execute("DELETE FROM docs WHERE rowid = ?", (row,))
            self.db.execute("DELETE FROM identities WHERE rowid = ?", (row,))


def check(body, bucket):
    """What a documents-contract hit carried: CRC-32 and byte length of the returned text, its byte range, and the bucket."""
    raw = body.encode("utf-8")
    return {"crc": zlib.crc32(raw), "len": len(raw), "start": 0, "end": len(raw), "bucket": bucket}


class SqliteFts5Docs(SqliteFts5):
    """Documents contract: the same contentless FTS5 index as the ids row plus a `docstore` table holding id, body and
    bucket. One statement ranks the top-100 rowids and joins them to their stored columns by primary key."""

    def build(self, documents):
        super().build(documents)
        db = sqlite3.connect(self.store / "fts.db")
        db.execute("CREATE TABLE docstore (rowid INTEGER PRIMARY KEY, id TEXT NOT NULL, body TEXT NOT NULL, bucket INTEGER NOT NULL)")
        with db:
            for i, d in enumerate(documents, 1):
                db.execute("INSERT INTO docstore(rowid, id, body, bucket) VALUES (?, ?, ?, ?)", (i, d["id"], d["text"], d.get("metadata", {}).get("bucket", 0)))
        db.commit()
        db.close()

    def search(self, expression):
        if not expression:
            return []
        # The measured-fastest of three fetch forms (join on the top-100 subquery, one IN statement, 100 point selects:
        # evidence run10/micro_fts.log).
        rows = self.db.execute(
            "SELECT d.id, t.rank, d.body, d.bucket FROM (SELECT rowid, rank FROM docs WHERE docs MATCH ? ORDER BY rank LIMIT ?) t "
            "JOIN docstore d ON d.rowid = t.rowid ORDER BY t.rank", (expression, K)).fetchall()
        return [(id_, -score, body, bucket) for id_, score, body, bucket in rows]

    def replace(self, document):
        super().replace(document)
        row = self.db.execute("SELECT rowid FROM identities WHERE id = ?", (document["id"],)).fetchone()[0]
        with self.db:
            self.db.execute("UPDATE docstore SET body = ? WHERE rowid = ?", (document["text"] + " benchmarkrevision", row))

    def delete(self, document):
        row = self.db.execute("SELECT rowid FROM identities WHERE id = ?", (document["id"],)).fetchone()[0]
        super().delete(document)
        with self.db:
            self.db.execute("DELETE FROM docstore WHERE rowid = ?", (row,))


class Tantivy:
    def __init__(self, store):
        self.store = Path(store)

    def _schema(self):
        import tantivy
        sb = tantivy.SchemaBuilder()
        sb.add_text_field("id", stored=True, tokenizer_name="raw")
        sb.add_text_field("body", stored=False, tokenizer_name="en_stem")
        return sb.build()

    def build(self, documents):
        import tantivy
        self.store.mkdir(parents=True)
        index = tantivy.Index(self._schema(), path=str(self.store))
        writer = index.writer(heap_size=200_000_000, num_threads=1)
        for d in documents:
            writer.add_document(tantivy.Document(id=d["id"], body=d["text"]))
        writer.commit()
        writer.wait_merging_threads()

    def open(self):
        import tantivy
        self.index = tantivy.Index(self._schema(), path=str(self.store))
        self.index.reload()
        self.searcher = self.index.searcher()

    def prepare(self, text):
        # Parsed inside the timer: every system receives the raw query string.
        query, _ = self.index.parse_query_lenient(" ".join(words(text)), ["body"])
        return query

    def search(self, query):
        hits = self.searcher.search(query, K).hits
        return [(self.searcher.doc(addr)["id"][0], score) for score, addr in hits]

    def _writer(self):
        return self.index.writer(heap_size=50_000_000, num_threads=1)

    def replace(self, document):
        import tantivy
        writer = self._writer()
        writer.delete_documents("id", document["id"])
        writer.add_document(tantivy.Document(id=document["id"], body=document["text"] + " benchmarkrevision"))
        writer.commit()
        self.index.reload()
        self.searcher = self.index.searcher()

    def delete(self, document):
        writer = self._writer()
        writer.delete_documents("id", document["id"])
        writer.commit()
        self.index.reload()
        self.searcher = self.index.searcher()


class TantivyDocs(Tantivy):
    """Documents contract: id, body and bucket are stored and every hit's stored document is read through
    `searcher.doc` (the binding's only document access; it exposes no doc-store cache setting)."""

    def _schema(self):
        import tantivy
        sb = tantivy.SchemaBuilder()
        sb.add_text_field("id", stored=True, tokenizer_name="raw")
        sb.add_text_field("body", stored=True, tokenizer_name="en_stem")
        sb.add_unsigned_field("bucket", stored=True, indexed=True)
        return sb.build()

    def build(self, documents):
        import tantivy
        self.store.mkdir(parents=True)
        index = tantivy.Index(self._schema(), path=str(self.store))
        writer = index.writer(heap_size=200_000_000, num_threads=1)
        for d in documents:
            writer.add_document(self._document(d["id"], d["text"], d.get("metadata", {}).get("bucket", 0)))
        writer.commit()
        writer.wait_merging_threads()

    @staticmethod
    def _document(id_, text, bucket):
        import tantivy
        document = tantivy.Document(id=id_, body=text)
        document.add_unsigned("bucket", bucket)
        return document

    def search(self, query):
        hits = self.searcher.search(query, K).hits
        out = []
        for score, addr in hits:
            d = self.searcher.doc(addr)
            out.append((d["id"][0], score, d["body"][0], d["bucket"][0]))
        return out

    def replace(self, document):
        import tantivy
        writer = self._writer()
        writer.delete_documents("id", document["id"])
        writer.add_document(self._document(document["id"], document["text"] + " benchmarkrevision", document.get("metadata", {}).get("bucket", 0)))
        writer.commit()
        self.index.reload()
        self.searcher = self.index.searcher()


class Whoosh:
    def __init__(self, store):
        self.store = Path(store)

    def _schema(self):
        from whoosh.analysis import StemmingAnalyzer
        from whoosh.fields import ID, TEXT, Schema
        return Schema(id=ID(stored=True, unique=True), body=TEXT(analyzer=StemmingAnalyzer()))

    def build(self, documents):
        from whoosh import index
        self.store.mkdir(parents=True)
        ix = index.create_in(str(self.store), self._schema())
        writer = ix.writer(procs=1, limitmb=512)
        for d in documents:
            writer.add_document(id=d["id"], body=d["text"])
        writer.commit()

    def open(self):
        from whoosh import index
        from whoosh.qparser import OrGroup, QueryParser
        self.ix = index.open_dir(str(self.store))
        self.searcher = self.ix.searcher()
        self.parser = QueryParser("body", self.ix.schema, group=OrGroup)

    def prepare(self, text):
        return " ".join(words(text))

    def search(self, text):
        if not text:
            return []
        hits = self.searcher.search(self.parser.parse(text), limit=K)
        return [(h["id"], h.score) for h in hits]

    def _reload(self):
        # refresh() reuses stale segment readers after deletion-only commits.
        self.searcher.close()
        self.searcher = self.ix.searcher()

    def replace(self, document):
        writer = self.ix.writer()
        writer.update_document(id=document["id"], body=document["text"] + " benchmarkrevision")
        writer.commit()
        self._reload()

    def delete(self, document):
        writer = self.ix.writer()
        writer.delete_by_term("id", document["id"])
        writer.commit()
        self._reload()


class Bm25s:
    def __init__(self, store):
        self.store = Path(store)

    def _tokens(self, texts):
        import bm25s
        import Stemmer
        return bm25s.tokenize(texts, stopwords="en", stemmer=Stemmer.Stemmer("english"), show_progress=False)

    def build(self, documents):
        import bm25s
        self.store.mkdir(parents=True)
        retriever = bm25s.BM25()
        retriever.index(self._tokens([d["text"] for d in documents]), show_progress=False)
        retriever.save(str(self.store))
        (self.store / "ids.json").write_text(json.dumps([d["id"] for d in documents]))

    def open(self):
        import bm25s
        import Stemmer
        self.retriever = bm25s.BM25.load(str(self.store), load_corpus=False)
        self.ids = json.loads((self.store / "ids.json").read_text())
        self.stemmer = Stemmer.Stemmer("english")

    def prepare(self, text):
        return " ".join(words(text))

    def search(self, text):
        import bm25s
        if not text:
            return []
        tokens = bm25s.tokenize([text], stopwords="en", stemmer=self.stemmer, show_progress=False)
        docs, scores = self.retriever.retrieve(tokens, k=min(K, len(self.ids)), show_progress=False, n_threads=1)
        return [(self.ids[int(i)], float(s)) for i, s in zip(docs[0], scores[0]) if s > 0]

    def replace(self, document):
        raise Unsupported("immutable index; any change is a full rebuild")

    def delete(self, document):
        raise Unsupported("immutable index; any change is a full rebuild")


class LancedbFts:
    """LanceDB native full-text index (Rust BM25, not the tantivy-backed variant): stemmed English, other settings default."""

    def __init__(self, store):
        self.store = Path(store)

    def build(self, documents):
        import lancedb
        import pyarrow as pa
        self.store.mkdir(parents=True)
        db = lancedb.connect(str(self.store / "db"))
        table = db.create_table("t", pa.table({"id": pa.array([d["id"] for d in documents], type=pa.string()),
                                               "text": pa.array([d["text"] for d in documents], type=pa.string())}))
        table.create_fts_index("text", use_tantivy=False, language="English", stem=True, remove_stop_words=True, replace=True)

    def open(self):
        import lancedb
        self.table = lancedb.connect(str(self.store / "db")).open_table("t")

    def prepare(self, text):
        return " ".join(words(text))

    def search(self, text):
        if not text:
            return []
        rows = self.table.search(text, query_type="fts", fts_columns="text").select(["id"]).limit(K).to_arrow()
        return list(zip(rows["id"].to_pylist(), rows["_score"].to_pylist()))

    def replace(self, document):
        import pyarrow as pa
        self.table.delete("id = '" + document["id"].replace("'", "''") + "'")
        self.table.add(pa.table({"id": [document["id"]], "text": [document["text"] + " benchmarkrevision"]}))
        self.table.optimize()

    def delete(self, document):
        self.table.delete("id = '" + document["id"].replace("'", "''") + "'")
        self.table.optimize()


class DuckdbFts:
    """DuckDB `fts` extension: BM25 over a static inverted index (porter stemmer, English stop words)."""

    def __init__(self, store):
        self.store = Path(store)

    def _connect(self):
        import duckdb
        con = duckdb.connect(str(self.store / "fts.duckdb"))
        con.execute("INSTALL fts; LOAD fts")
        return con

    def build(self, documents):
        import pyarrow as pa
        self.store.mkdir(parents=True)
        con = self._connect()
        try:
            con.execute("CREATE TABLE docs(id VARCHAR PRIMARY KEY, text VARCHAR)")
            for lo in range(0, len(documents), 20000):
                batch = documents[lo: lo + 20000]
                con.register("warm_ingest_batch", pa.table({"id": [d["id"] for d in batch], "text": [d["text"] for d in batch]}))
                try:
                    con.execute("INSERT INTO docs SELECT * FROM warm_ingest_batch")
                finally:
                    con.unregister("warm_ingest_batch")
            con.execute("PRAGMA create_fts_index('docs', 'id', 'text', stemmer='porter', stopwords='english', lower=1, overwrite=1)")
            con.execute("CHECKPOINT")
        finally:
            con.close()

    def open(self):
        self.con = self._connect()

    def prepare(self, text):
        return " ".join(words(text))

    def search(self, text):
        if not text:
            return []
        return self.con.execute("SELECT id, score FROM (SELECT id, fts_main_docs.match_bm25(id, ?, fields := 'text') AS score FROM docs) "
                                "WHERE score IS NOT NULL ORDER BY score DESC, id LIMIT ?", [text, K]).fetchall()

    def replace(self, document):
        raise Unsupported("the fts index is static; any change needs create_fts_index again (full rebuild)")

    def delete(self, document):
        raise Unsupported("the fts index is static; any change needs create_fts_index again (full rebuild)")


SYSTEMS = {"sqlite-fts5": SqliteFts5, "tantivy-py": Tantivy, "tantivy": Tantivy, "whoosh": Whoosh, "bm25s": Bm25s, "lancedb-fts": LancedbFts, "duckdb-fts": DuckdbFts,
           "sqlite-fts5-prev": SqliteFts5, "tantivy-prev": Tantivy, "sqlite-fts5-docs": SqliteFts5Docs, "tantivy-py-docs": TantivyDocs}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("mode", choices=["build", "serve", "ops"])
    p.add_argument("--system", required=True)
    p.add_argument("--input", required=True)
    p.add_argument("--store", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--passes", type=int, default=3)
    p.add_argument("--ops", type=int, default=100)
    p.add_argument("--t0-ns", type=int, default=T_IMPORT)
    a = p.parse_args()
    data = json.loads(Path(a.input).read_text())
    system = SYSTEMS[a.system](a.store)
    if a.mode == "build":
        start = time.perf_counter()
        system.build(data["documents"])
        report = {"documents": len(data["documents"]), "ingest_s": time.perf_counter() - start}
    elif a.mode == "serve":
        start = time.perf_counter()
        getattr(system, "open_serving", system.open)()
        open_s = time.perf_counter() - start
        queries = data["queries"]
        # One contract for every system: the raw query string goes in, the engine's own query analysis and
        # construction (`prepare`) are inside the timed region, and the top-100 (id, score) pairs come out.
        prepared = [q["text"] for q in queries]

        def search(text):
            return system.search(system.prepare(text))
        search(prepared[0])
        cold_start_s = (time.time_ns() - a.t0_ns) / 1e9
        for text in prepared[:16]:
            search(text)
        rows, passes = [], []
        documents = a.system.endswith("-docs")
        for n in range(a.passes):
            ns = []
            for q, text in zip(queries, prepared):
                t = time.perf_counter_ns()
                found = search(text)
                ns.append(time.perf_counter_ns() - t)
                if n == 0 and not documents:
                    rows.append({"id": q["id"], "results": [{"documentId": i, "lexicalScore": s} for i, s in found]})
            passes.append(ns)
        if documents:
            # Untimed: what each hit carried, for the orchestrator to compare with the corpus.
            for q, text in zip(queries, prepared):
                rows.append({"id": q["id"], "results": [{"documentId": i, "lexicalScore": s, "check": check(body, bucket)} for i, s, body, bucket in search(text)]})
        report = {"open_s": open_s, "cold_start_s": cold_start_s, "rows": rows, "passes_ns": passes}
    else:
        system.open()
        docs = data["documents"]
        report = {"count": a.ops}
        for name, subset in (("replace", docs[: a.ops]), ("delete", docs[a.ops: 2 * a.ops])):
            try:
                t = time.perf_counter()
                for d in subset:
                    getattr(system, name)(d)
                report[name] = {"supported": True, "per_op_ms_durable": (time.perf_counter() - t) / a.ops * 1e3}
            except Unsupported as e:
                report[name] = {"supported": False, "reason": str(e)}
    report["peak_rss_bytes_self"] = peak_rss()
    Path(a.out).write_text(json.dumps(report, allow_nan=False))


if __name__ == "__main__":
    main()
