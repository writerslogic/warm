# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Numerical difference between WARM's pure-Rust embeddings and the reference sentence-transformers fp32 model.

  embed_parity.py --work DIR --corpus nfcorpus --dataset beir-nfcorpus-dev --revision SHA --out FILE [--docs 300]

Embeds the first N documents and every query of a prepared dataset with sentence-transformers (PyTorch CPU, fp32) and
compares with the shared vectors file: cosine between the two vectors of each text (min, mean) and, over the first
queries, the overlap of the top-10 documents by exact cosine under each vector set.
"""

import argparse
import json
from pathlib import Path

import numpy as np


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--work", required=True)
    p.add_argument("--corpus", required=True)
    p.add_argument("--dataset", required=True)
    p.add_argument("--revision", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--docs", type=int, default=300)
    a = p.parse_args()
    from sentence_transformers import SentenceTransformer
    work = Path(a.work)
    corpus = json.loads((work / "corpus" / a.corpus / "input.json").read_text())["documents"][: a.docs]
    queries = json.loads((work / a.dataset / "serve-input.json").read_text())["queries"]
    dv = np.fromfile(work / "corpus" / a.corpus / "docs.f32", dtype="<f4").reshape(-1, 384)[: a.docs]
    qv = np.fromfile(work / a.dataset / "queries.f32", dtype="<f4").reshape(len(queries), 384)
    model = SentenceTransformer("ibm-granite/granite-embedding-30m-english", revision=a.revision, device="cpu")
    model.max_seq_length = 512
    rd = model.encode([d["text"] for d in corpus], normalize_embeddings=True, batch_size=16)
    rq = model.encode([q["text"] for q in queries], normalize_embeddings=True, batch_size=16)
    cos_d, cos_q = (dv * rd).sum(1), (qv * rq).sum(1)
    agree = []
    for i in range(min(len(queries), 100)):
        a10 = set(np.argsort(-(dv @ qv[i]))[:10])
        b10 = set(np.argsort(-(rd @ rq[i]))[:10])
        agree.append(len(a10 & b10) / 10)
    out = {"documents": len(corpus), "queries": len(queries), "doc_cosine_min": float(cos_d.min()), "doc_cosine_mean": float(cos_d.mean()),
           "query_cosine_min": float(cos_q.min()), "query_cosine_mean": float(cos_q.mean()), "max_abs_component_diff_docs": float(np.abs(dv - rd).max()),
           "top10_overlap_mean_over_queries_within_sample": float(np.mean(agree)), "reference_revision": a.revision}
    Path(a.out).write_text(json.dumps(out, indent=1))
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
