# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Best QPS per recall target from an ood_local.py console log (4-decimal recall), registered grid only."""
import re
import sys
rows = []
for line in open(sys.argv[1]):
    m=re.match(r'ef=(\d+) rerank=(\d+): (.*)',line.strip())
    if not m: continue
    ef,rr=int(m[1]),int(m[2])
    for part in m[3].split(' | '):
        a, rest = part.split(' ')[0], part.split(' ')[1]
        rec,qps=rest.split('@')
        rows.append((a,ef,rr,float(rec),float(qps)))
reg=lambda ef,rr: ef<=1000 and rr in (100,200,400) and ef>=40
for a in sorted({r[0] for r in rows}):
    print('ARM',a,'n',len([r for r in rows if r[0]==a]),'maxrec',max(r[3] for r in rows if r[0]==a and reg(r[1],r[2])))
    for t in (0.90,0.95,0.99):
        el=[r for r in rows if r[0]==a and reg(r[1],r[2]) and r[3]>=t]
        b=max(el,key=lambda r:r[4]) if el else None
        print('  ',t,b and f"ef{b[1]}/rr{b[2]} rec {b[3]} qps {b[4]:.0f}")
