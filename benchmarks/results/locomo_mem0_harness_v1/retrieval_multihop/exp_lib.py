import json, collections
from common import *
qs = load_questions(); base = json.load(open('base200.json'))
tt = {ci: turn_table(ci) for ci in range(10)}
by_dia = {ci: {t['dia']: t for t in tt[ci]} for ci in tt}; f2t = {ci: {t['fid']: t for t in tt[ci]} for ci in tt}
rows0 = json.load(open('phase0_rows.json'))
def gold_pos(q): return [by_dia[q['ci']][d]['pos'] for d in rows0[q['id']]['ev2'] if d in by_dia[q['ci']]]
def positions(q, hits): return [f2t[q['ci']][i]['pos'] for i in hits]
def metrics(q, pos):
    g = gold_pos(q); s = set(pos)
    return {'all': float(all(p in s for p in g)), 'rec': float(np.mean([p in s for p in g]))}
def neighbour_expand(ci, pos, S, K=200):
    T = tt[ci]; out = []; seen = set()
    def add(p):
        if p not in seen: seen.add(p); out.append(p)
    # seeds keep their order; neighbours go directly after their seed; later (non-seed) hits follow
    seeds = pos[:S]; sset = set(seeds)
    for p in seeds:
        add(p)
        for n in (p - 1, p + 1):
            if 0 <= n < len(T) and T[n]['sess'] == T[p]['sess']: add(n)
    for p in pos[S:]: add(p)
    # truncating from the tail: seeds' neighbours stay ahead of non-seed hits
    return out[:K]
def boot_ci(d, n=10000, seed=0):
    d = np.asarray(d, float); d = d[~np.isnan(d)]; rng = np.random.default_rng(seed)
    m = d[rng.integers(0, len(d), (n, len(d)))].mean(1); return d.mean(), np.percentile(m, 2.5), np.percentile(m, 97.5)
