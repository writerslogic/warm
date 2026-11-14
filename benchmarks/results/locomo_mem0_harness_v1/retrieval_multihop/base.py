# Fresh baseline at the original server flags (max-top-k 200), cached query embeddings; two serial passes for latency.
import json
from common import *
qs = load_questions(); ce = CacheOnly(); embs = {q['id']: ce.embed(q['q']) for q in qs}
out = {}; lat = [[], []]
for p in range(2):
    for q in qs:
        r, ms = recall(q['ci'], q['q'], embs[q['id']], 200); lat[p].append(ms)
        if p == 0: out[q['id']] = [(h['id'], h['score']) for h in r]
        else: assert [h['id'] for h in r] == [i for i, _ in out[q['id']]]
json.dump(out, open('base200.json', 'w')); json.dump(lat, open('base200_latency_ms.json', 'w'))
print('base200 done; pass2 identical to pass1 for all; latency ms pass2 p50 %.2f mean %.2f p95 %.2f' % (np.percentile(lat[1], 50), np.mean(lat[1]), np.percentile(lat[1], 95)))
