#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Held-out report: G1 vs SPAN_V scores with bootstrap 95% intervals (4000 resamples, seed 7), paired differences,
pre-reader latency and Gemini cost per question.
usage: report.py --g1 judged/g1_{}/per_question.jsonl --base judged/span_v_{}/per_question.jsonl --stages-dir run/  ({} = web, enterprise)"""
import argparse, json, random, statistics as S
import fileio  # noqa: E402
from pathlib import Path
import common

ap = argparse.ArgumentParser(); ap.add_argument("--g1", required=True); ap.add_argument("--base", required=True); ap.add_argument("--stages-dir", required=True); a = ap.parse_args()
Q = common.questions()
def load(p):
    o = {}
    for d in ('web', 'enterprise'):
        for l in fileio.lines(p.format(d)): x = json.loads(l); o[x['question_id']] = float(x['score'])
    return o
g = load(a.g1); b = load(a.base)
ids = sorted(g); ab = lambda i: Q[i]['question_type'].endswith('-abs')
SEL = {'overall': lambda i: True, 'answerable': lambda i: not ab(i), 'abstention': ab, 'web': lambda i: Q[i]['domain'] == 'web', 'enterprise': lambda i: Q[i]['domain'] == 'enterprise'}
def ci(f, s, B=4000, seed=7):
    r = random.Random(seed); v = sorted(100 * sum(f(r.choice(s)) for _ in s) / len(s) for _ in range(B)); return v[int(.025 * B)], v[int(.975 * B)]
for k, fn in SEL.items():
    s = [i for i in ids if fn(i)]; m = 100 * sum(g[i] for i in s) / len(s); lo, hi = ci(lambda i: g[i], s); mb = 100 * sum(b[i] for i in s) / len(s); dl, dh = ci(lambda i: g[i] - b[i], s)
    print(f'{k:11s} n={len(s):3d} G1 {m:5.1f} [{lo:.1f},{hi:.1f}]  SPAN_V {mb:5.1f}  diff {m-mb:+5.1f} [{dl:+.1f},{dh:+.1f}]')
print('wins', sum(g[i] > b[i] for i in ids), 'losses', sum(g[i] < b[i] for i in ids))
D = Path(a.stages_dir)
L = lambda f: {json.loads(l)['id']: json.loads(l) for l in fileio.lines(D / f)}
C, V, X, O, A, P2 = L('span_v.jsonl'), L('verify.jsonl'), L('crux.jsonl'), L('opt.jsonl'), L('anscheck.jsonl'), L('pass2.jsonl')
lat = [C[i]['stats']['seconds'] + max(V[i]['seconds'], X[i]['seconds'], O[i]['seconds'] if i in O else 0) + (P2[i]['stats']['seconds'] if i in P2 else 0) for i in ids]
lat2 = [l + A[i]['seconds'] for l, i in zip(lat, ids)]
cost = [C[i]['cost_usd'] + V[i]['cost_usd'] + X[i]['cost_usd'] + (O[i]['cost_usd'] if i in O else 0) + (P2[i]['cost_usd'] if i in P2 else 0) + A[i]['cost_usd'] for i in ids]
q = lambda v, p: sorted(v)[int(p * len(v))]
print('memory.query latency (pre-reader) median %.1f p95 %.1f; incl. answer check median %.1f p95 %.1f; Gemini cost/q $%.4f' % (S.median(lat), q(lat, .95), S.median(lat2), q(lat2, .95), S.mean(cost)))
