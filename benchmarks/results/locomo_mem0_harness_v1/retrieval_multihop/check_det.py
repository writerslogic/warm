import json
from common import *
qs = load_questions(); res = stored(); deep = json.load(open('deep1000.json'))
tt = {ci: turn_table(ci) for ci in range(10)}; f2d = {(ci, t['fid']): t['dia'] for ci in tt for t in tt[ci]}
setsame = 0; jac = []; diffs = []
rows = json.load(open('phase0_rows.json'))
agree_all = 0; new_all = {}
for q in qs:
    st = [x['id'] for x in res[q['id']]['retrieval']['search_results']]; nw = [i for i, _ in deep[q['id']][:200]]
    setsame += set(st) == set(nw); jac.append(len(set(st) & set(nw)) / 200)
    ev = rows[q['id']]['ev2']; got = {f2d[(q['ci'], i)] for i in nw}
    new_all[q['id']] = all(e in got for e in ev)
    agree_all += new_all[q['id']] == rows[q['id']]['all2']
print('same set', setsame, 'of', len(qs), 'mean overlap', np.mean(jac), 'min', min(jac), 'all-gold flag agrees', agree_all)
for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'):
    s = [q['id'] for q in qs if q['cat'] == c]
    print(c, 'stored all-gold %.3f fresh %.3f' % (np.mean([rows[i]['all2'] for i in s]), np.mean([new_all[i] for i in s])))
