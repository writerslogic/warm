import json, sys, numpy as np
TARGETS=(0.95,0.99,0.995)
def frontier(rows):
    pts=sorted(((r['recall'],r['median_qps']) for r in rows),key=lambda p:-p[1])  # by qps desc
    best=-1; out=[]
    for rec,q in pts:
        if rec>best: out.append((rec,q)); best=rec
    return sorted(out)  # recall asc, qps desc
def at(fr,t):
    rs=[p[0] for p in fr]; qs=[p[1] for p in fr]
    if t>rs[-1]: return None
    j=next(i for i,r in enumerate(rs) if r>=t)
    if j==0: return qs[0]
    r0,r1,q0,q1=rs[j-1],rs[j],qs[j-1],qs[j]
    w=(t-r0)/(r1-r0) if r1>r0 else 1
    return float(np.exp(np.log(q0)*(1-w)+np.log(q1)*w))
for p in sys.argv[1:]:
    with open(p) as fh: d=json.load(fh)
    print(p.split('/')[-1])
    base=None
    for a in d['arms']:
        fr=frontier(d['rows'][a['name']])
        v=[at(fr,t) for t in TARGETS]
        if base is None: base=v
        print(f"{a['name']:36s}", ' '.join(f"{(x if x else 0):7.0f}" for x in v), ' ratio-to-first', ' '.join(f"{(x/b if x and b else 0):5.2f}" for x,b in zip(v,base)))
