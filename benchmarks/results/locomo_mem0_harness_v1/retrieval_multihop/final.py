import json
from exp_lib import *
exec(open('stageB.py').read().split('# ingestTime')[0])
print('--- all-gold in top-k from the deeper k=1000 list (approximate: pool normalisation differs from k=200)')
deep = json.load(open('deep1000.json'))
for K in (200, 300, 400, 600):
    r = []
    for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'):
        ids = [q for q in qs if q['cat'] == c]
        r.append(f'{c} {np.mean([metrics(q, positions(q, [i for i, _ in deep[q["id"]][:K]]))["all"] for q in ids]):.3f}')
    print(f'k={K}:', ' | '.join(r))
chg = []
for q in qs:
    bp = positions(q, [i for i, _ in base[q['id']]]); n = neighbour_expand(q['ci'], bp, 20)
    if set(bp) != set(n): chg.append(q['id'])
json.dump(chg, open('n20_changed_ids.json', 'w'))
print('N20 informational: questions whose top-200 set changes', len(chg), 'of', len(qs), 'multi-hop', sum(1 for i in chg if next(q for q in qs if q['id'] == i)['cat'] == 'multi-hop'))
print('cost at 0.027 per question: changed %.2f USD, all 1540 %.2f USD' % (len(chg) * 0.027, 1540 * 0.027))
