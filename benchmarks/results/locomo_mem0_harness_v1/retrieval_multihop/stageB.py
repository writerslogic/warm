# Stage B: the single selected configuration (N20) on held-out conversations 3-9, run once.
from exp_lib import *
ho = [q for q in qs if q['ci'] >= 3]; res = {'base': {}, 'N20': {}}; changed = {}
for q in ho:
    bp = positions(q, [i for i, _ in base[q['id']]]); np_ = neighbour_expand(q['ci'], bp, 20)
    res['base'][q['id']] = metrics(q, bp); res['N20'][q['id']] = metrics(q, np_); changed[q['id']] = set(bp) != set(np_)
out = {}
for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop', 'ALL'):
    ids = [q['id'] for q in ho if c == 'ALL' or q['cat'] == c]
    da = [res['N20'][i]['all'] - res['base'][i]['all'] for i in ids]; dr = [res['N20'][i]['rec'] - res['base'][i]['rec'] for i in ids]
    a = boot_ci(da); r = boot_ci(dr); out[c] = {'n': len(ids), 'all_diff': a, 'rec_diff': r, 'base_all': float(np.mean([res['base'][i]['all'] for i in ids])), 'n20_all': float(np.mean([res['N20'][i]['all'] for i in ids])), 'gained': int(sum(x > 0 for x in da)), 'lost': int(sum(x < 0 for x in da))}
    print(f'{c:12s} n={len(ids):3d} all-gold {out[c]["base_all"]:.3f} -> {out[c]["n20_all"]:.3f} diff {a[0]:+.3f} [{a[1]:+.3f},{a[2]:+.3f}] gained {out[c]["gained"]} lost {out[c]["lost"]}  recall diff {r[0]:+.3f} [{r[1]:+.3f},{r[2]:+.3f}]')
json.dump(out, open('stageB.json', 'w'), indent=1)
# ingestTime order check against dataset order (needed for an engine implementation on the existing store)
import concurrent.futures as cf
def ing(a):
    ci, t = a; return post('get', {'id': t['fid']}, f'locomo_{ci}_{RUN}')['fact']['ingestTime']
bad = 0; ties = 0
for ci in range(10):
    with cf.ThreadPoolExecutor(8) as ex: it = list(ex.map(ing, [(ci, t) for t in tt[ci]]))
    bad += sum(b < a for a, b in zip(it, it[1:])); ties += sum(b == a for a, b in zip(it, it[1:]))
print('ingestTime vs dataset order: decreasing steps', bad, 'ties', ties)
