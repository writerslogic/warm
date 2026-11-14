# Deep rankings (k=1000) and serial k=200 latency/determinism check on the existing store. Query embeddings come from the cache only.
import json, sys
from concurrent.futures import ThreadPoolExecutor
from common import *
qs = load_questions(); ce = CacheOnly(); res = stored()
embs = {q['id']: ce.embed(q['q']) for q in qs}
print('cached query embeddings', len(embs))
tt = {ci: turn_table(ci) for ci in range(10)}
fid2dia = {(ci, t['fid']): t['dia'] for ci in tt for t in tt[ci]}
# determinism: k=200 serial vs stored top-200, plus latency
same = 0; lat = []; unknown = 0
for q in qs:
    r, ms = recall(q['ci'], q['q'], embs[q['id']], 200); lat.append(ms)
    ids = [h['id'] for h in r]; st = [x['id'] for x in res[q['id']]['retrieval']['search_results']]
    same += ids == st; unknown += sum((q['ci'], i) not in fid2dia for i in ids)
print('top-200 identical to stored run:', same, 'of', len(qs), 'unmapped ids', unknown, 'serial k=200 latency ms p50 %.1f mean %.1f p95 %.1f' % (np.percentile(lat, 50), np.mean(lat), np.percentile(lat, 95)))
json.dump(lat, open('baseline_latency_ms.json', 'w'))
def deep(q):
    r, _ = recall(q['ci'], q['q'], embs[q['id']], 1000)
    return q['id'], [(h['id'], h.get('score')) for h in r]
with ThreadPoolExecutor(4) as ex: out = dict(ex.map(deep, qs))
json.dump(out, open('deep1000.json', 'w'))
print('deep rankings', len(out), 'mean length', np.mean([len(v) for v in out.values()]))
