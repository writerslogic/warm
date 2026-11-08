# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Per-query misses of one arm: build with connectivity report, list queries whose tolerant recall is below 1.

Usage: concentration.py DATA.hdf5 ESCAPE(0|1) REACH(0|1)"""
import sys, time, json, numpy as np, h5py
from writerslogic_warm import VectorIndex
data, escape, reach = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
h=h5py.File(data); train=h['train'][:]; q=h['test'][:]; nb=h['neighbors'][:]; dist=h['distances'][:]; learn=h['learn'][:]
order=np.random.default_rng(24301).permutation(len(learn))[:32768]
idx=VectorIndex('angular',degree=64,build_ef=500,threads=1,code_bits=8)
t=time.time()
idx.fit(train,training_queries=np.ascontiguousarray(learn[order]),escape_edges=bool(escape),reach_edges=bool(reach),report=True)
print('build',time.time()-t,'connectivity',idx.connectivity(),flush=True)
for ef,rr in [(1000,400),(2000,1000)]:
    idx.set_query_arguments(ef,rr)
    miss=[]; strict=[]
    for i,v in enumerate(q):
        ids=np.asarray(idx.query(v,100))
        d=1-train[ids]@v
        thr=dist[i,99]+1e-3
        ok=int((d<=thr).sum()); miss.append(100-ok)
        strict.append(len(set(ids.tolist())&set(nb[i].tolist())))
        if ok<100:
            gone=[int(g) for g in nb[i] if g not in set(ids.tolist())]
            unreach=set(idx.connectivity()['unreachable_sample'])
            print(f'  q{i} ef{ef}: missing GT ids {gone[:8]} ; in unreachable sample: {[g for g in gone if g in unreach]}')
    miss=np.array(miss)
    print(f'ef{ef} rr{rr}: recall {1-miss.sum()/miss.size/100:.6f} missing {miss.sum()} of {miss.size*100}; queries with a miss {int((miss>0).sum())}; strict {np.mean(strict)/100:.5f}',flush=True)
    print(' top misses (query, count):',[(int(i),int(miss[i])) for i in np.argsort(-miss)[:5] if miss[i]>0])
