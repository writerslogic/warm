import json, glob, collections, numpy as np, os
P = '/Volumes/A/.hms-lb/locomo-20261114/pilot'; H = os.path.expanduser('~')
res = {}
for f in glob.glob(H + '/pilot-out/all/predicted_all/conv*_q*.json'):
    e = json.load(open(f)); res[e['question_id']] = e
m0 = {e['question_id']: e for e in json.load(open('/Volumes/A/.hms-lb/locomo-20261114/mem0-benchmarks/f/results/platform/locomo_results.json'))['evaluations']}
data = json.load(open('/Volumes/A/.hms-lb/locomo-20261114/locomo10.json'))
turn = {(ci, t['dia_id']): t['text'] for ci, c in enumerate(data) for k, v in c['conversation'].items() if isinstance(v, list) for t in v}
rng = np.random.default_rng(0)
def ci_(d):
    m = d[rng.integers(0, len(d), (10000, len(d)))].mean(1); return d.mean(), np.percentile(m, 2.5), np.percentile(m, 97.5)
ids = sorted(i for i in res if i in m0)
S = lambda e: e['cutoff_results']['top_200']['score']
print(f'WARM questions completed {len(res)} of 1540; matched to Mem0 {len(ids)}')
def line(name, sub):
    if not sub: return
    w = np.array([S(res[i]) for i in sub]); m = np.array([S(m0[i]) for i in sub]); d = ci_(w - m)
    print(f'{name:14s} n={len(sub):4d} WARM {w.mean()*100:5.1f}% Mem0 {m.mean()*100:5.1f}% diff {d[0]*100:+5.1f} pp [{d[1]*100:+5.1f}, {d[2]*100:+5.1f}]')
line('overall', ids)
for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'): line(c, [i for i in ids if res[i]['category_name'] == c])
print('per conversation')
for k in range(10): line(f'conv {k}', [i for i in ids if res[i]['conversation_idx'] == k])
def cls(e):
    mem = [x['memory'] for x in e['retrieval']['search_results']]; ev = e['evidence']
    pres = [any(turn.get((e['conversation_idx'], d), '\0') in x for x in mem) for d in ev]
    return 'no evidence listed' if not ev else 'all gold present (reader/judge)' if all(pres) else 'some gold absent (partial retrieval miss)' if any(pres) else 'no gold present (retrieval miss)'
out = {}
for name, wok in (('WARM wrong, Mem0 right', False), ('Mem0 wrong, WARM right', True)):
    sel = [i for i in ids if (S(res[i]) >= .5) == wok and (S(m0[i]) >= .5) != wok]
    cnt = collections.Counter((res[i]['category_name'], cls(res[i])) for i in sel)
    print(f'{name}: {len(sel)} (gold-evidence presence is measured in WARM\'s 200 items)')
    for k, v in sorted(cnt.items()): print('  ', k, v)
    tot = collections.Counter(cls(res[i]) for i in sel); print('   totals', dict(tot))
    out[name] = [{'id': i, 'cat': res[i]['category_name'], 'class': cls(res[i]), 'q': res[i]['question'], 'gold': res[i]['ground_truth_answer'], 'warm': res[i]['cutoff_results']['top_200']['generated_answer'], 'mem0': m0[i]['cutoff_results']['top_200'].get('generated_answer')} for i in sel]
json.dump(out, open(P + '/misses_full.json', 'w'), indent=1)
u = {}
for f in ('calib-usage.jsonl', 'full-usage.jsonl', 'all-usage.jsonl'):
    if os.path.exists(f'{P}/{f}'):
        for l in open(f'{P}/{f}'): r = json.loads(l); u[r['key']] = r
for kind in ('answer', 'judge'):
    k = [x for x in u.values() if x['kind'] == kind]
    print(kind, 'paid calls logged', len(k), 'prompt/call', round(np.mean([x['prompt'] for x in k])), 'completion/call', round(np.mean([x['completion'] for x in k])), 'reasoning/call', round(np.mean([x['reasoning'] for x in k])), 'length-truncated', sum(x['finish'] == 'length' for x in k))
a = [json.loads(l) for l in open(P + '/all-usage.jsonl')] if os.path.exists(P + '/all-usage.jsonl') else []
print('additional spend this task (all-usage.jsonl) usd', round(sum(x['usd'] for x in a), 4), 'calls', len(a))
print('models returned', sorted({x['model_returned'] for x in u.values()}), 'requested', sorted({x['model_requested'] for x in u.values()}))
