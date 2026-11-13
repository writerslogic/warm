# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
import argparse
import hashlib
import importlib.util
import json
import time
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

p=argparse.ArgumentParser();p.add_argument('--module',required=True);p.add_argument('--output',required=True);args=p.parse_args()
module=Path(args.module).resolve()
spec=importlib.util.spec_from_file_location('writerslogic_warm',module);warm=importlib.util.module_from_spec(spec);spec.loader.exec_module(warm)
root=Path('/opt/warm-pr898')
queries=np.asarray(pq.read_table(root/'dataset/cohere/cohere_medium_1m/test.parquet', columns=['emb'])['emb'].to_pylist(),dtype=np.float32)
ix=warm.VectorIndex(metric='angular',degree=32,build_ef=300,threads=1)
ix.load(root/'index-064/vector_bench_test/index.warm')
results=[]
for ef in (120,150,200,400,600,800):
 ix.set_query_arguments(ef=ef,rerank=ef)
 for q in queries: ix.query(q,100)
 for rep in range(5):
  order=np.random.default_rng(898+rep).permutation(len(queries))
  out=np.empty((len(queries),100),dtype=np.int64)
  start=time.perf_counter_ns()
  for i in order: out[i]=ix.query(queries[i],100)
  elapsed=time.perf_counter_ns()-start
  row=dict(ef=ef,rep=rep,ns_per_query=elapsed/len(queries),digest=hashlib.sha256(out.tobytes()).hexdigest())
  print(json.dumps(row),flush=True); results.append(row)
Path(args.output).write_text(json.dumps({'module':str(module),'module_sha256':hashlib.sha256(module.read_bytes()).hexdigest(),'results':results},indent=2)+'\n')
