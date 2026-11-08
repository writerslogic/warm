"""ann-benchmarks wrapper for WARM used by the agent-A campaign (adds an arm tag in the name, optional
build cache keyed by data fingerprint + build parameters + wheel tag, and the extra query knobs).
Index builds are deterministic, so a cached build serves the same graph a fresh fit would."""
import hashlib, os
import numpy as np
from writerslogic_warm import VectorIndex
from ..base.module import BaseANN

CACHE = os.environ.get("WARM_BUILD_CACHE", "")
BUILD_TAG = os.environ.get("WARM_BUILD_TAG", "")


def cache_file(X, metric, degree, build_ef, code_bits, residual_bits, compact):
    h = hashlib.sha256()
    h.update(f"{X.shape}{metric}{degree}{build_ef}{code_bits}{residual_bits}{compact}{BUILD_TAG}".encode())
    h.update(np.ascontiguousarray(X[:: max(1, len(X) // 997)]).tobytes())
    return os.path.join(CACHE, h.hexdigest()[:24] + ".warm")


class WARM(BaseANN):
    def __init__(self, metric, method_param):
        if metric not in ("angular", "euclidean"):
            raise NotImplementedError(metric)
        self.metric = metric
        self.degree = method_param["degree"]
        self.build_ef = method_param["build_ef"]
        self.code_bits = method_param.get("code_bits", 8)
        self.residual_bits = method_param.get("residual_bits", 8)
        self.compact = method_param.get("compact", False)
        self.arm = method_param.get("arm", "")
        self.base = (f"warm[{self.arm}](degree={self.degree}, build_ef={self.build_ef}, "
                     f"code_bits={self.code_bits}, residual_bits={self.residual_bits}, compact={self.compact}")
        self.name = self.base + ")"
        self.index = None

    def fit(self, X):
        X = np.ascontiguousarray(X, dtype=np.float32)
        kw = dict(degree=self.degree, build_ef=self.build_ef, code_bits=self.code_bits,
                  residual_bits=self.residual_bits, compact_adjacency=self.compact)
        self.index = VectorIndex(self.metric, **kw)
        if CACHE:
            f = cache_file(X, self.metric, self.degree, self.build_ef, self.code_bits, self.residual_bits, self.compact)
            if os.path.exists(f):
                self.index.load(f)
                return
            self.index.fit(X)
            self.index.save(f + ".tmp")
            os.replace(f + ".tmp", f)
            return
        self.index.fit(X)

    def set_query_arguments(self, ef, rerank, patience=0, reservoir=0):
        self.index.set_query_arguments(ef, rerank, patience, reservoir)
        self.name = self.base + f", ef={ef}, rerank={rerank}, patience={patience}, reservoir={reservoir})"

    def query(self, v, n):
        return self.index.query(v, n)

    def get_additional(self):
        return {"index_bytes": self.index.index_bytes}

    def done(self):
        self.index = None
