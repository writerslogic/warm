# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import hashlib
import json
import os
import time
from pathlib import Path

import numpy as np
import writerslogic_warm as warm

results = []
for dim in (128, 784):
    rng = np.random.default_rng(20261109)
    n, nq = 32768, 128
    data = rng.integers(0, 256, (n, dim), dtype=np.uint8)
    queries = rng.integers(0, 256, (nq, dim), dtype=np.uint8)
    count = np.ones(n, dtype=np.int64) + (np.arange(n) % 4 == 0)
    ptr = np.r_[0, np.cumsum(count)]
    labels = np.zeros(ptr[-1], dtype=np.int32)
    labels[ptr[:-1][count == 2] + 1] = 1
    index = warm.FilteredIndex(data, ptr, labels, labels=2, ivf_min=n + 1, threads=1)
    index.set_search(brute_max=n + 1)
    query_count = np.ones(nq, dtype=np.int64) + (np.arange(nq) % 2 == 0)
    qp = np.r_[0, np.cumsum(query_count)]
    ql = np.zeros(qp[-1], dtype=np.int32)
    ql[qp[:-1][query_count == 2] + 1] = 1
    for deleted in (False, True):
        if deleted:
            for row in range(0, n, 17):
                assert index.delete(row)
        expected = index.search(queries, qp, ql, 10)
        times = []
        for rep in range(5):
            start = time.perf_counter_ns()
            actual = index.search(queries, qp, ql, 10)
            times.append((time.perf_counter_ns() - start) / nq)
            np.testing.assert_array_equal(actual, expected)
        results.append({"dim": dim, "rows": n, "queries": nq, "deleted": deleted,
                        "ns_per_query": times, "digest": hashlib.sha256(expected.tobytes()).hexdigest()})
Path(os.environ["OUTPUT"]).write_text(json.dumps({"module": warm.__file__, "results": results}, indent=2) + "\n")
