# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Which systems exist, how each is isolated, and the static operational facts.

Facts are recorded into every results file next to the measurements, each with a `basis`
("measured" with a results file, or "documentation" with the URL), so the generated table never
carries a claim that has no source in the results directory.
"""

import platform

from . import jars

WARM = "warm"
# The WARM text arm runs twice: `warm` ingests with `--encode lexical` (BM25 index only, the
# like-for-like row against the lexical comparators) and `warm-eager` with the default eager
# encoding (chunk hypervectors built at ingest as well). Each build repeat is its own cold
# process in both rows. `warm-eager` is never a comparator in a verdict (same family).
WARM_EAGER = "warm-eager"
WARM_TEXT_ENCODE = {WARM: "lexical", WARM_EAGER: "eager"}
# The tantivy crate behind the native arm (examples/competitive_text_tantivy.rs, pinned in Cargo.toml) is the
# version tantivy-py 0.27.0 is built against; the example reports it in its build output.
TANTIVY_CRATE = "0.27.0"
# Result files written before the rename key the Python binding as `tantivy`.
LEGACY_KEYS = {"hms": WARM, "hms-eager": WARM_EAGER, "hms-server": "warm-server", "tantivy": "tantivy-py"}

# kind: ann | text.  env: pinned environment (envs.PINS key).  worker: key inside the worker module.
SYSTEMS = {
    WARM: {"display": "WARM", "kind": "both", "env": None, "package": "writerslogic-warm", "family": "warm"},
    WARM_EAGER: {"display": "WARM (eager hypervectors, informational)", "kind": "text", "env": None, "package": "writerslogic-warm", "family": "warm"},
    "hnswlib": {"display": "hnswlib", "kind": "ann", "env": "hnswlib", "package": "hnswlib", "family": "hnsw"},
    "hnswlib-cpp": {"display": "hnswlib (native C++)", "kind": "ann", "env": "hnswlib", "package": "hnswlib", "family": "hnsw"},
    "faiss-hnsw": {"display": "FAISS HNSW", "kind": "ann", "env": "faiss", "package": "faiss-cpu", "family": "hnsw"},
    "faiss-flat": {"display": "FAISS Flat (exact)", "kind": "ann", "env": "faiss", "package": "faiss-cpu", "family": "exact"},
    "faiss-ivfpq": {"display": "FAISS IVF-PQ (+refine)", "kind": "ann", "env": "faiss", "package": "faiss-cpu", "family": "ivf"},
    "usearch": {"display": "USearch", "kind": "ann", "env": "usearch", "package": "usearch", "family": "hnsw"},
    "annoy": {"display": "Annoy", "kind": "ann", "env": "annoy", "package": "annoy", "family": "trees"},
    "sqlite-vec": {"display": "sqlite-vec (exact)", "kind": "ann", "env": "sqlite-vec", "package": "sqlite-vec", "family": "exact"},
    "lancedb": {"display": "LanceDB", "kind": "ann", "env": "lancedb", "package": "lancedb", "family": "ivf"},
    "chroma": {"display": "Chroma", "kind": "ann", "env": "chroma", "package": "chromadb", "family": "hnsw"},
    "qdrant-local": {"display": "Qdrant (embedded local mode)", "kind": "ann", "env": "qdrant", "package": "qdrant-client", "family": "exact"},
    "milvus-lite": {"display": "Milvus Lite", "kind": "ann", "env": "milvus", "package": "pymilvus", "family": "hnsw"},
    "duckdb-vss": {"display": "DuckDB vss", "kind": "ann", "env": "duckdb", "package": "duckdb", "family": "hnsw"},
    "sqlite-fts5": {"display": "SQLite FTS5", "kind": "text", "env": "stdlib", "package": "sqlite3", "family": "lexical"},
    "lucene": {"display": f"Apache Lucene {jars.VERSION} (JVM)", "kind": "text", "env": None, "package": "lucene-core", "family": "lexical"},
    # The Python binding and the native crate are separate rows: the binding adds a language boundary and a
    # per-hit Python object that the native crate does not pay, so neither stands in for the other.
    "tantivy-py": {"display": "tantivy-py 0.27.0 (CPython binding)", "kind": "text", "env": "tantivy", "package": "tantivy", "family": "lexical"},
    "tantivy-rs": {"display": f"tantivy {TANTIVY_CRATE} (native Rust)", "kind": "text", "env": None, "package": "tantivy", "family": "lexical"},
    # Version controls: the previous pin of each current comparator, never in a default run or a verdict.
    # Select them with `--systems` to measure the version effect inside the same interleaved rounds.
    "sqlite-fts5-prev": {"display": "SQLite FTS5 (previous pin, SQLite 3.50.4)", "kind": "text-control", "env": "stdlib-prev", "package": "sqlite3", "family": "lexical"},
    "lucene-prev": {"display": f"Apache Lucene {jars.PREVIOUS} (JVM, previous pin)", "kind": "text-control", "env": None, "package": "lucene-core", "family": "lexical"},
    "tantivy-prev": {"display": "tantivy-py 0.26.2 (previous pin)", "kind": "text-control", "env": "tantivy-prev", "package": "tantivy", "family": "lexical"},
    "whoosh": {"display": "Whoosh", "kind": "text", "env": "whoosh", "package": "whoosh-reloaded", "family": "lexical"},
    "bm25s": {"display": "bm25s", "kind": "text", "env": "bm25s", "package": "bm25s", "family": "lexical"},
}

# Where each system's query is timed. Binding overhead is part of what a user of that package
# pays, but it differs by language, so it is printed next to every timing row.
TIMING_BOUNDARY = {
    WARM: "native Rust API call in the engine process (no language binding); text: BM25 index only (`--encode lexical`); ANN: public-bench loop",
    WARM_EAGER: "native Rust API call in the engine process (no language binding); text: BM25 index plus chunk hypervectors at ingest (`--encode eager`)",
    "hnswlib": "CPython binding call per query",
    "hnswlib-cpp": "native C++ searchKnn call per query from a C++ driver process (no Python in the timed region); same index file and queries as the hnswlib row",
    "faiss-hnsw": "CPython (SWIG) call per query", "faiss-flat": "CPython (SWIG) call per query",
    "faiss-ivfpq": "CPython (SWIG) call per query", "usearch": "CPython binding call per query", "annoy": "CPython binding call per query",
    "sqlite-vec": "CPython sqlite3 call per query", "lancedb": "CPython + Arrow call per query (multi-threaded Rust runtime inside)",
    "chroma": "CPython client call per query (Rust core inside)", "qdrant-local": "CPython client call per query (embedded local mode)",
    "milvus-lite": "CPython client call per query (embedded server process)", "duckdb-vss": "CPython duckdb call per query",
    "sqlite-fts5": "CPython sqlite3 call per query (query string built outside the timer)", "tantivy-py": "CPython tantivy-py call per query (query parsed outside the timer; stored id fetched per hit through Python objects)",
    "tantivy-rs": "native Rust API call in its own process (query parsed outside the timer; id read from a fast field per hit), same process model as WARM",
    "whoosh": "pure Python", "bm25s": "CPython + NumPy (single-threaded retrieve)", "lucene": "in-process JVM API call, after untimed JIT warm-up passes to steady state; query analyzed outside the timer, ids from an array built at open",
}

# JVM flags per Lucene phase. serve: a small fixed heap and the serial collector (the index is memory-mapped, so the
# heap holds only the id table, prepared queries and per-query garbage), compiler left at its default tiered C2, and no
# perf-data mapping; build and ops: heap sized from the machine, parallel collector, the usual indexing setup.
LUCENE_JVM_FLAGS = {
    "serve": ["-Xms64m", "-Xmx128m", "-XX:+UseSerialGC", "-XX:-UsePerfData"],
    "build": ["-XX:MaxRAMPercentage=25", "-XX:+UseParallelGC", "-XX:-UsePerfData"],
    "ops": ["-XX:MaxRAMPercentage=25", "-XX:+UseParallelGC", "-XX:-UsePerfData"],
}

# Recorded into every text results file for the systems that ran, so a reader sees the settings each comparator was
# given and how the timed region was kept to the search itself.
COMPARATOR_SETTINGS = {
    "lucene": {"jvm_flags": LUCENE_JVM_FLAGS, "query_cache": "disabled (IndexSearcher.setQueryCache(null))",
               "query_analysis": "outside the timed region (one Query per query, built before the passes)",
               "id_resolution": "String[] indexed by docid, built at open; no stored-fields read in the timed region",
               "warmup": "untimed whole passes until two consecutive passes agree within 5 percent (3 to 30), count in warmup_passes",
               "similarity": "BM25Similarity default (k1 1.2, b 0.75); index force-merged to one segment"},
    "sqlite-fts5": {"query": "SELECT rowid, rank FROM docs WHERE docs MATCH ? ORDER BY rank LIMIT 100 (no join, no tie-break)",
                    "id_resolution": "rowid-to-id dict loaded at open, outside the timed region",
                    "pragmas": {"query_only": "ON", "mmap_size": 1073741824, "cache_size": "-65536 (64 MiB)", "temp_store": "MEMORY"},
                    "tokenizer": "porter unicode61", "index": "optimize run at build"},
    "tantivy-py": {"query": "parse_query_lenient outside the timed region; searcher.search(query, 100)",
                   "id_resolution": "stored field fetched per hit through Python objects (the binding's only document access)",
                   "analyzer": "en_stem, raw id field", "writer": "1 thread, 200 MB heap"},
    "tantivy-rs": {"query": "QueryParser::parse_query_lenient outside the timed region; TopDocs::with_limit(100)",
                   "id_resolution": "id fast field (column read) per hit", "analyzer": "en_stem, raw id field",
                   "writer": "1 thread, 200 MB heap", "crate": f"tantivy {TANTIVY_CRATE}", "process": "one process per phase, same as WARM"},
}
COMPARATOR_SETTINGS["lucene-prev"] = COMPARATOR_SETTINGS["lucene"]
COMPARATOR_SETTINGS["sqlite-fts5-prev"] = COMPARATOR_SETTINGS["sqlite-fts5"]
COMPARATOR_SETTINGS["tantivy-prev"] = COMPARATOR_SETTINGS["tantivy-py"]

_DOC = "documentation, not exercised here"

FACTS = {
    WARM: {
        "service_needed": {"value": "no", "basis": "measured", "source": "benchmarks/results/security_matrix_v2.json",
                           "note": "in-process library; 0 egress sockets in the security matrix"},
        "encrypted_at_rest": {"value": "yes", "basis": "measured", "source": "benchmarks/results/security_matrix_v2.json",
                              "note": "AES-GCM with Argon2id (feature `security`); encrypted QPS ratio 0.929 of plain; not enabled in the rows above"},
    },
    "hnswlib": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "hnswlib-cpp": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "faiss-hnsw": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "faiss-flat": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "faiss-ivfpq": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "usearch": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "annoy": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "sqlite-vec": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no (SQLCipher is a separate build)", "basis": _DOC}},
    "lancedb": {"service_needed": {"value": "no (embedded)", "basis": _DOC}, "encrypted_at_rest": {"value": "no built-in", "basis": _DOC}},
    "chroma": {"service_needed": {"value": "no (PersistentClient)", "basis": _DOC}, "encrypted_at_rest": {"value": "no built-in", "basis": _DOC}},
    "qdrant-local": {"service_needed": {"value": "no (local mode; server needed for indexed ANN)", "basis": _DOC},
                     "encrypted_at_rest": {"value": "no built-in", "basis": _DOC}},
    "milvus-lite": {"service_needed": {"value": "no (embedded)", "basis": _DOC}, "encrypted_at_rest": {"value": "no built-in", "basis": _DOC}},
    "duckdb-vss": {"service_needed": {"value": "no", "basis": _DOC},
                   "encrypted_at_rest": {"value": "database-file encryption (DuckDB 1.4+); not exercised with the vss index", "basis": _DOC}},
    "sqlite-fts5": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no (SQLCipher is a separate build)", "basis": _DOC}},
    "tantivy-py": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "tantivy-rs": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "whoosh": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "bm25s": {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}},
    "lucene": {"service_needed": {"value": "no (library; needs a JVM)", "basis": _DOC}, "encrypted_at_rest": {"value": "no (filesystem-level only)", "basis": _DOC}},
}

# Systems named in the brief that were not run, with the reason recorded in every results file.
NOT_RUN = {
    "mem0": "agent-memory systems belong to the next harness stage (plug in through the same adapter interface)",
    "zep": "agent-memory systems belong to the next harness stage",
    "letta": "agent-memory systems belong to the next harness stage",
    "langmem": "agent-memory systems belong to the next harness stage",
}

FACTS[WARM_EAGER] = FACTS[WARM]

SYSTEMS.update({
    "ngt": {"display": "NGT (ONNG)", "kind": "ann", "env": "ngt", "package": "ngt", "family": "graph"},
    "scann": {"display": "ScaNN", "kind": "ann", "env": "scann", "package": "scann", "family": "ivf"},
    "pynndescent": {"display": "PyNNDescent", "kind": "ann", "env": "pynndescent", "package": "pynndescent", "family": "graph"},
    "voyager": {"display": "Voyager", "kind": "ann", "env": "voyager", "package": "voyager", "family": "hnsw"},
    "diskann": {"display": "DiskANN (in-memory Vamana)", "kind": "ann", "env": "diskann", "package": "diskannpy", "family": "graph"},
})
TIMING_BOUNDARY.update({
    "ngt": "CPython (pybind11) call per query", "scann": "CPython (pybind11) call per query; the ScaNN searcher runs its own thread pool for training only",
    "pynndescent": "CPython + numba-compiled kernel per query (JIT compile inside cold start)",
    "voyager": "CPython 3.12 binding call per query", "diskann": "CPython 3.11 binding call per query (diskannpy StaticMemoryIndex)",
})
FACTS.update({k: {"service_needed": {"value": "no", "basis": _DOC}, "encrypted_at_rest": {"value": "no", "basis": _DOC}} for k in ("ngt", "scann", "pynndescent", "voyager", "diskann")})
# Reasons that apply to one OS only, recorded in results files produced on that OS.
_PLATFORM_NOT_RUN = {"scann": {"Darwin": "no macOS wheel for scann 1.4.2 (manylinux_2_27 x86_64 and aarch64 only); Linux x86_64 runs it"},
                     "diskann": {"Darwin": "no macOS wheel for diskannpy 0.7.0 (manylinux_2_28 x86_64 and win_amd64, CPython 3.9-3.11 only); Linux x86_64 runs it"}}
NOT_RUN.update({k: v[platform.system()] for k, v in _PLATFORM_NOT_RUN.items() if platform.system() in v})

ANN_SYSTEMS = [k for k, v in SYSTEMS.items() if v["kind"] == "ann"]
TEXT_SYSTEMS = [k for k, v in SYSTEMS.items() if v["kind"] == "text"]
TEXT_CONTROL_SYSTEMS = [k for k, v in SYSTEMS.items() if v["kind"] == "text-control"]

# Server-mode systems (own table sections, measured against warm-server): server_registry.py.
from .server_registry import SERVER_FACTS, SERVER_NOT_RUN, SERVER_SYSTEMS, SERVER_TIMING_BOUNDARY  # noqa: E402

SYSTEMS.update(SERVER_SYSTEMS)
FACTS.update(SERVER_FACTS)
TIMING_BOUNDARY.update(SERVER_TIMING_BOUNDARY)
NOT_RUN.update(SERVER_NOT_RUN)


# What "durable" means for one replace or delete, per system, from the adapter code that measures it. A replace/delete
# row is compared only between systems of the same class. "incremental-durable-commit": durable on return, cost
# proportional to the change (WARM appends one fsynced journal frame; a SQLite or search-index commit logs the change).
# "full-index-rewrite": the adapter saves the whole index file and fsyncs it per operation, because the library has no
# incremental durable API; that is a different contract and is never compared with an incremental commit.
INCREMENTAL = "incremental-durable-commit"
REWRITE = "full-index-rewrite"
UNAUDITED = "unaudited"
COMMIT_CONTRACT = {
    WARM: (INCREMENTAL, "one fsynced journal frame per commit (F_FULLFSYNC on macOS)"),
    WARM_EAGER: (INCREMENTAL, "one fsynced journal frame per commit (F_FULLFSYNC on macOS)"),
    "sqlite-vec": (INCREMENTAL, "one SQLite transaction per operation with fullfsync on (ann_worker SqliteVec)"),
    "sqlite-fts5": (INCREMENTAL, "one SQLite transaction per operation with fullfsync on"),
    "tantivy-py": (INCREMENTAL, "writer commit per operation (segment and meta.json synced)"),
    "tantivy-rs": (INCREMENTAL, "writer commit per operation (segment and meta.json synced)"),
    "tantivy-prev": (INCREMENTAL, "writer commit per operation (segment and meta.json synced)"),
    "lucene": (INCREMENTAL, "IndexWriter commit per operation (FileChannel.force)"),
    "lucene-prev": (INCREMENTAL, "IndexWriter commit per operation (FileChannel.force)"),
    "whoosh": (INCREMENTAL, "writer commit per operation"),
    "hnswlib": (REWRITE, "save_index of the whole index plus fsync per operation; the library has no incremental durable API"),
    "faiss-hnsw": (REWRITE, "write_index of the whole index plus fsync per operation"),
    "faiss-flat": (REWRITE, "write_index of the whole index plus fsync per operation"),
    "faiss-ivfpq": (REWRITE, "write_index of the whole index plus fsync per operation"),
    "usearch": (REWRITE, "save of the whole index plus fsync per operation"),
    "ngt": (REWRITE, "save of the whole index plus fsync per operation"),
    "voyager": (REWRITE, "save of the whole index plus fsync per operation"),
}


def commit_contract(name):
    """(class, description) of a system's durable replace/delete; unaudited systems match nothing."""
    return COMMIT_CONTRACT.get(LEGACY_KEYS.get(name, name), (UNAUDITED, "adapter's flush semantics not audited as equivalent to an incremental commit"))


def system(name):
    """Resolve immutable measurement IDs without changing source-file key paths."""
    return SYSTEMS[LEGACY_KEYS.get(name, name)]
