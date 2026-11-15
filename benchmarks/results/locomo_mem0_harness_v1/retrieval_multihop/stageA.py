# Stage A: all registered candidates on dev conversations 0-2 only.
import sys
from exp_lib import *
ce = CacheOnly(); dev = [q for q in qs if q['ci'] <= 2]
cfg = {'N10': ('N', 10), 'N20': ('N', 20), 'N50': ('N', 50), 'C400': ('C', 400), 'C1000': ('C', 1000)}
res = {'base': {}}; lists = {}
for q in dev: res['base'][q['id']] = metrics(q, positions(q, [i for i, _ in base[q['id']]]))
for name, (kind, v) in cfg.items():
    res[name] = {}
    for q in dev:
        bp = positions(q, [i for i, _ in base[q['id']]])
        if kind == 'N': pos = neighbour_expand(q['ci'], bp, v)
        else:
            r = post('recall', {'query': q['q'], 'k': 200, 'embedding': ce.embed(q['q']), 'candidateLimit': v}, f"locomo_{q['ci']}_{RUN}")['results']
            pos = positions(q, [h['id'] for h in r])
        res[name][q['id']] = metrics(q, pos); lists[(name, q['id'])] = pos
cats = ('multi-hop', 'temporal', 'open-domain', 'single-hop')
out = {}
for name in cfg:
    print(name)
    out[name] = {}
    for c in cats + ('ALL',):
        ids = [q['id'] for q in dev if c == 'ALL' or q['cat'] == c]
        da = [res[name][i]['all'] - res['base'][i]['all'] for i in ids]; dr = [res[name][i]['rec'] - res['base'][i]['rec'] for i in ids]
        a = boot_ci(da); r = boot_ci(dr); out[name][c] = {'n': len(ids), 'all_diff': a, 'rec_diff': r, 'changed_all': int(sum(x != 0 for x in da))}
        print(f'  {c:12s} n={len(ids):3d} all-gold {np.mean([res["base"][i]["all"] for i in ids]):.3f} -> {np.mean([res[name][i]["all"] for i in ids]):.3f} diff {a[0]:+.3f} [{a[1]:+.3f},{a[2]:+.3f}]  recall diff {r[0]:+.3f} [{r[1]:+.3f},{r[2]:+.3f}]')
json.dump(out, open('stageA.json', 'w'), indent=1)
