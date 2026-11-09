"""VIBE wrapper for WARM (writerslogic-warm), the quantized graph index.

The index is a proximity graph whose traversal reads 8-bit (default) or 4-bit codes of the mean-centered rows;
the `rerank` best candidates are re-scored with an 8-bit residual code. Cosine and normalized
data are searched by angle, euclidean data by Euclidean distance, and ip data by inner product:
the graph is built on rows lifted by one coordinate (MIPS reduced to Euclidean search) and served
directly on the original coordinates by negative inner product (`direct_ip`).

On out-of-distribution datasets (config `ood: true` and a learn split in the HDF5 file) VIBE calls
`fit_ood` with the dataset's official learn split. The build adds query-aware edges from a seeded
sample of LEARN_QUERIES rows of that split (all rows when the split is smaller); the learn
neighbors, test queries and ground truth are never used.
"""
import os

# Back the index arrays with transparent huge pages (a no-op where the kernel refuses them).
# Set before the import so revisions where the advice is off by default take it too.
os.environ.setdefault("WARM_QGRAPH_HUGEPAGES", "1")

import numpy as np
from writerslogic_warm import VectorIndex

from ..base.module import BaseANN

# Sample of the learn split that routes the query-aware build: numpy default_rng(LEARN_SEED)
# permutation, first LEARN_QUERIES rows. The native build also caps training queries at 32768.
LEARN_QUERIES = 32768
# Exact neighbours of the learn queries cost queries x rows x dimensions multiply-adds on one thread;
# a request for more than LEARN_QUERIES learn rows is cut to what fits this budget (never below LEARN_QUERIES).
LEARN_BUDGET = 2e13
LEARN_SEED = 24301

METRICS = {"cosine": "angular", "normalized": "angular", "euclidean": "euclidean", "ip": "ip"}


class WARM(BaseANN):
    def __init__(self, metric, degree, build_ef, code_bits=8, query_aware=True, escape_edges=False,
                 reach_edges=False, projected_degree=32, training_neighbors=100, training_ef=0,
                 learn_queries=LEARN_QUERIES):
        if metric not in METRICS:
            raise NotImplementedError(f"warm does not support the {metric} distance")
        self.metric = metric
        self.degree = degree
        self.build_ef = build_ef
        if code_bits not in (4, 8):
            raise ValueError("code_bits must be 4 or 8")
        self.code_bits = code_bits
        self.query_aware = bool(query_aware)
        self.escape_edges = bool(escape_edges)
        self.reach_edges = bool(reach_edges)
        self.projected_degree = projected_degree
        self.training_neighbors = training_neighbors
        self.training_ef = training_ef
        self.learn_query_limit = int(learn_queries)
        self.connectivity = None
        self.trained = False
        self.learn_queries = 0
        self.ef = None
        self.rerank = None
        self.patience = 0
        self.reservoir = 0
        self.index = None

    def fit(self, X):
        # In-distribution sets have no learn split: the plain graph.
        self.trained = False
        self.learn_queries = 0
        self._fit(X, None)

    def fit_ood(self, X, X_learn, X_learn_neighbors):
        # Out-of-distribution sets: the dataset's official learn split routes the graph (never
        # the test queries).
        self.trained = self.query_aware and X_learn is not None and len(X_learn) > 0
        if not self.trained:
            self.learn_queries = 0
            self._fit(X, None)
            return
        affordable = max(LEARN_QUERIES, int(LEARN_BUDGET // (len(X) * len(X[0]))))
        order = np.random.default_rng(LEARN_SEED).permutation(len(X_learn))[:min(self.learn_query_limit, affordable)]
        self.learn_queries = len(order)
        self._fit(X, X_learn[order])

    def _fit(self, X, queries):
        self.index = VectorIndex(METRICS[self.metric], degree=self.degree, build_ef=self.build_ef,
                                 threads=1, code_bits=self.code_bits, direct_ip=self.metric == "ip")
        learning = None if queries is None else np.ascontiguousarray(queries, dtype=np.float32)
        # Only passed when asked for, so a wheel without the option still builds the default.
        options = {}
        if self.escape_edges:
            options["escape_edges"] = True
        if self.reach_edges:
            options["reach_edges"] = True
        if queries is not None:
            options.update(projected_degree=self.projected_degree, training_neighbors=self.training_neighbors,
                           training_ef=self.training_ef, training_limit=max(self.learn_query_limit, len(queries)))
        self.index.fit(np.ascontiguousarray(X, dtype=np.float32), training_queries=learning, **options)
        self.connectivity = self.index.connectivity() if options.keys() & {"escape_edges", "reach_edges"} else None

    def save_index(self, path):
        self.index.save(str(path))

    def load_index(self, path):
        # The graph file carries the build; the constructor arguments only fix the search settings.
        self.trained = self.query_aware
        self.index = VectorIndex(METRICS[self.metric], degree=self.degree, build_ef=self.build_ef,
                                 threads=1, code_bits=self.code_bits, direct_ip=self.metric == "ip")
        self.index.load(str(path))

    def set_query_arguments(self, ef, rerank=None, patience=0, reservoir=0):
        if rerank is None:
            arguments = tuple(ef)
            if len(arguments) not in (2, 3, 4):
                raise ValueError("query arguments must contain ef, rerank and optional patience and reservoir")
            ef, rerank, patience, reservoir = (arguments + (0, 0))[:4]
        if reservoir:
            self.index.set_query_arguments(ef, rerank, patience, reservoir)
        elif patience == 0:
            self.index.set_query_arguments(ef, rerank)
        else:
            self.index.set_query_arguments(ef, rerank, patience)
        self.ef = ef
        self.rerank = rerank
        self.patience = patience
        self.reservoir = reservoir

    def query(self, v, n):
        return self.index.query(v, n)

    def __str__(self):
        precision = "" if self.code_bits == 4 else f", code_bits={self.code_bits}"
        training = f", learn_queries={self.learn_queries}" if self.trained else ""
        escape = (", escape_edges=True" if self.escape_edges else "") + (", reach_edges=True" if self.reach_edges else "")
        if self.trained and (self.projected_degree, self.training_neighbors, self.training_ef, self.learn_query_limit) != (32, 100, 0, LEARN_QUERIES):
            escape += f", projected_degree={self.projected_degree}, training_neighbors={self.training_neighbors}, training_ef={self.training_ef}, learn_queries={self.learn_query_limit}"
        early_stop = (f", patience={self.patience}" if self.patience > 0 else "") + (
            f", reservoir={self.reservoir}" if self.reservoir > 0 else "")
        return f"WARM(metric={self.metric}, degree={self.degree}, build_ef={self.build_ef}, ef={self.ef}, rerank={self.rerank}{precision}{training}{escape}{early_stop})"
