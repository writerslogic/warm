"""ann-benchmarks wrapper for WARM used by the agent-A campaign (adds an arm tag in the name, optional
build cache keyed by full fit data, parameters and loaded native artifact, and extra query knobs).
Cache hits report load work; they are not measurements of a fresh build."""
import hashlib
import importlib.machinery
import json
import os
from pathlib import Path
import sys
import tempfile
import numpy as np
from writerslogic_warm import VectorIndex
from ..base.module import BaseANN

CACHE = os.environ.get("WARM_BUILD_CACHE", "")
BUILD_TAG = os.environ.get("WARM_BUILD_TAG", "")


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def runtime_identity():
    modules = {name: Path(module.__file__).resolve() for name, module in tuple(sys.modules.items())
               if (name == "writerslogic_warm" or name.startswith("writerslogic_warm."))
               and getattr(module, "__file__", None)}
    if not any(str(path).endswith(suffix) for path in modules.values()
               for suffix in importlib.machinery.EXTENSION_SUFFIXES):
        raise ValueError("cannot identify the loaded WARM native artifact")
    return {name: file_hash(path) for name, path in sorted(modules.items())}


def cache_file(X, metric, degree, build_ef, code_bits, residual_bits, compact):
    if X.ndim != 2 or X.dtype != np.float32 or not X.flags.c_contiguous:
        raise ValueError("cache identity requires the exact contiguous f32 fit matrix")
    identity = dict(schema=2, shape=X.shape, dtype=X.dtype.str, metric=metric, degree=degree,
                    build_ef=build_ef, code_bits=code_bits, residual_bits=residual_bits,
                    compact=compact, build_tag=BUILD_TAG, runtime=runtime_identity(),
                    wrapper_sha256=file_hash(__file__),
                    build_environment={key: value for key, value in sorted(os.environ.items())
                                       if key.startswith("WARM_QGRAPH_") or key == "RAYON_NUM_THREADS"})
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(",", ":")).encode())
    digest.update(memoryview(X).cast("B"))
    return os.path.join(CACHE, digest.hexdigest() + ".warm")


def cache_load_or_build(path, index, X):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        index.load(str(path))
        return dict(hit=True, built=False, raced=False)
    index.fit(X)
    raced = False
    with tempfile.TemporaryDirectory(dir=path.parent, prefix=path.name + ".") as directory:
        temporary = Path(directory) / "index.warm"
        index.save(str(temporary))
        with temporary.open("rb") as stream:
            os.fsync(stream.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError:
            raced = True
            index.load(str(path))
    return dict(hit=False, built=True, raced=raced)


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
        self.cache_state = dict(hit=False, built=True, raced=False)

    def fit(self, X):
        X = np.ascontiguousarray(X, dtype=np.float32)
        kw = dict(degree=self.degree, build_ef=self.build_ef, code_bits=self.code_bits,
                  residual_bits=self.residual_bits, compact_adjacency=self.compact)
        self.index = VectorIndex(self.metric, **kw)
        if CACHE:
            f = cache_file(X, self.metric, self.degree, self.build_ef, self.code_bits, self.residual_bits, self.compact)
            self.cache_state = cache_load_or_build(f, self.index, X)
            return
        self.cache_state = dict(hit=False, built=True, raced=False)
        self.index.fit(X)

    def set_query_arguments(self, ef, rerank, patience=0, reservoir=0):
        self.index.set_query_arguments(ef, rerank, patience, reservoir)
        self.name = self.base + f", ef={ef}, rerank={rerank}, patience={patience}, reservoir={reservoir})"

    def query(self, v, n):
        return self.index.query(v, n)

    def get_additional(self):
        return {"index_bytes": self.index.index_bytes,
                **{f"build_cache_{key}": value for key, value in self.cache_state.items()}}

    def done(self):
        self.index = None
