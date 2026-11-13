# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Untimed optimistic feasibility bound; this does not implement or prove a pruning kernel."""
import hashlib
import json
import struct
from pathlib import Path

import numpy as np
import polars as pl
import writerslogic_warm as warm

root = Path('/Volumes/A/.hms-lb/vdbbench')
out = root / 'pr898-perf'
path = root / 'index/vector_bench_test/index.warm'
with path.open('rb') as stream:
    assert stream.read(8) == b'HMSVGR01'
    header = struct.unpack('<19Q', stream.read(19 * 8))
    sections = np.frombuffer(stream.read(15 * 16), dtype='<u8').reshape(15, 2).copy()
n, dim, bits, residual_bits, stride = header[1], header[2], header[4], header[5], header[6]
assert (dim, bits, residual_bits) == (768, 4, 8)

def section(number, dtype, shape):
    offset, length = map(int, sections[number])
    assert np.prod(shape) * np.dtype(dtype).itemsize == length
    return np.memmap(path, mode='r', offset=offset, dtype=dtype, shape=shape)

codes = section(5, np.int8, (n, stride))
residual = section(7, np.int8, (n, int(sections[7, 1]) // n))
scales = section(6, '<f4', (n,))
res_scales = section(8, '<f4', (n,))
original = section(9, '<u4', (n,))
inverse = np.empty(n, dtype=np.uint32)
inverse[original] = np.arange(n, dtype=np.uint32)
queries = np.asarray(pl.read_parquet(root / 'dataset/cohere/cohere_medium_1m/test.parquet')['emb'].to_list(), dtype=np.float32)
selection = np.load(out / 'query-split.npz')['selection']
index = warm.VectorIndex('angular', degree=32, build_ef=300, threads=1)
index.load(path)
records = []
for ef in (150, 400, 800):
    index.set_query_arguments(ef=ef, rerank=ef)
    for number in selection:
        query = queries[number]
        ids = index.query(query, ef)
        stored = inverse[ids]
        packed = codes[stored, :dim // 2].astype(np.uint8)
        primary = np.empty((ef, dim), dtype=np.int8)
        primary[:, 0::2] = ((packed & 15) ^ 8).astype(np.int8) - 8
        primary[:, 1::2] = ((packed >> 4) ^ 8).astype(np.int8) - 8
        primary = primary.astype(np.float64) * scales[stored, None].astype(np.float64)
        secondary = residual[stored, :dim].astype(np.float64) * res_scales[stored, None].astype(np.float64)
        q = query.astype(np.float64)
        sq = np.float32(np.max(np.abs(query)) / np.float32(127))
        values = query / sq
        qc = (np.copysign(np.floor(np.abs(values) + np.float32(.5)), values)).astype(np.int8)
        coarse_q = qc.astype(np.float64) * float(sq)
        coarse = primary @ coarse_q
        primary_scores = primary @ q
        fine = primary_scores + secondary @ q
        cutoff = fine[99]
        residual_bound = np.linalg.norm(secondary, axis=1) * np.linalg.norm(q)
        full_bound = residual_bound + np.linalg.norm(primary, axis=1) * np.linalg.norm(q - coarse_q)
        # This safety margin is diagnostic, not a substitute for outward-rounded proof.
        margin = 1e-5 * max(1., float(np.max(np.abs(fine))))
        assert np.all(fine <= coarse + full_bound + margin)
        assert np.all(fine <= primary_scores + residual_bound + margin)
        records.append(dict(ef=ef, query=int(number), candidates=ef,
            skip_without_rows=int(np.count_nonzero(coarse + full_bound + margin < cutoff)),
            skip_residual_after_primary=int(np.count_nonzero(primary_scores + residual_bound + margin < cutoff))))
    subset = [r for r in records if r['ef'] == ef]
    print(json.dumps(dict(ef=ef, queries=len(subset),
        skip_fraction=float(np.mean([r['skip_without_rows']/ef for r in subset])),
        skip_residual_fraction=float(np.mean([r['skip_residual_after_primary']/ef for r in subset])))), flush=True)
(out / 'rerank-bound-probe.json').write_text(json.dumps(dict(
    kind='untimed optimistic bound with oracle final cutoff; not production pruning or performance evidence',
    source=str(path), selection_sha256=hashlib.sha256(selection.tobytes()).hexdigest(), records=records), indent=2) + '\n')
