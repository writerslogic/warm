"""ann-benchmarks wrapper for WARM's PartitionIndex (IVF + SOAR + 4-bit PQ LUT16 scan + exact f32 reorder).
Angular data is unit-normalised (cosine order == Euclidean order of unit vectors)."""
import numpy as np
from writerslogic_warm import PartitionIndex
from ..base.module import BaseANN


class WARMIVF(BaseANN):
    def __init__(self, metric, method_param):
        if metric not in ("angular", "euclidean"):
            raise NotImplementedError(metric)
        self.metric = metric
        self.lists = method_param["lists"]
        self.pq_m = method_param["pq_m"]
        self.reorder = method_param["reorder"]
        self.arm = method_param.get("arm", "")
        self.threads = method_param.get("threads", 1)
        self.base = f"warmivf[{self.arm}](lists={self.lists}, pq_m={self.pq_m}, reorder={self.reorder}"
        self.name = self.base + ")"
        self.index = None

    def _prep(self, X):
        X = np.ascontiguousarray(X, dtype=np.float32)
        if self.metric == "angular":
            X = X / np.maximum(np.linalg.norm(X, axis=1, keepdims=True), 1e-30)
        return np.ascontiguousarray(X, dtype=np.float32)

    def fit(self, X):
        X = self._prep(X)
        n, d = X.shape
        self.index = PartitionIndex.create(d, lists=self.lists, train_at=n, nprobe=1, reorder=self.reorder,
                                           pq_m=self.pq_m, threads=self.threads)
        for s in range(0, n, 100000):
            self.index.insert(np.arange(s, min(n, s + 100000), dtype=np.int64), X[s:s + 100000])
        self.index.search(X[:1], 1)  # first search trains the centres and the codebook
        assert self.index.stats()["trained"]

    def set_query_arguments(self, nprobe):
        self.index.set_nprobe(nprobe)
        self.name = self.base + f", nprobe={nprobe})"

    def query(self, v, n):
        q = np.ascontiguousarray(v, dtype=np.float32)
        if self.metric == "angular":
            q = q / max(float(np.linalg.norm(q)), 1e-30)
        return self.index.search(q[None, :], n)[0]

    def done(self):
        self.index = None
