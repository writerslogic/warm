# Phase 0: reproduce the all-gold-in-top-200 classification from stored retrieval outputs (no server, no network).
import json, collections, re
from common import *
res = stored(); qs = {q['id']: q for q in load_questions()}
turn = {(ci, t[0]): t[2] for ci in range(10) for t in turns_of(ci)}
rows = {}
for i, q in qs.items():
    e = res[i]; mem = [x['memory'] for x in e['retrieval']['search_results']]
    pres = [any(turn.get((q['ci'], d), '\0') in m for m in mem) for d in e['evidence'] if isinstance(d, str)] if False else None
    ev = eval(e['evidence']) if isinstance(e['evidence'], str) else e['evidence']
    pres = [any(turn.get((q['ci'], d), '\0') in m for m in mem) for d in ev]   # parity with analyze_full.py: unparsable ids count as absent
    ev2 = [x.strip() for d in ev for x in re.split(r'[;,]', d) if x.strip()]
    pres2 = [any(turn.get((q['ci'], d), '\0') in m for m in mem) for d in ev2]
    rows[i] = {'cat': q['cat'], 'ci': q['ci'], 'n_ev': len(ev), 'n_pres': sum(pres), 'all': all(pres), 'any': any(pres), 'ev2': ev2, 'n_ev2': len(ev2), 'n_pres2': sum(pres2), 'all2': all(pres2), 'any2': any(pres2), 'bad_ids': sum(d not in [] and (q['ci'], d) not in turn for d in ev)}
print('questions with evidence', len(rows), 'questions with an unparsable evidence entry', sum(r['bad_ids']>0 for r in rows.values()), 'of which multi-hop', sum(r['bad_ids']>0 and r['cat']=='multi-hop' for r in rows.values()))
for name, sel in (('all', lambda r: True), ('dev 0-2', lambda r: r['ci'] <= 2), ('held-out 3-9', lambda r: r['ci'] >= 3)):
    print(name)
    for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'):
        s = [r for r in rows.values() if r['cat'] == c and sel(r)]
        print(f'  {c:12s} n={len(s):4d} all-gold {sum(r["all"] for r in s)/len(s):.3f} evidence-recall {sum(r["n_pres"] for r in s)/sum(r["n_ev"] for r in s):.3f} partial {sum((not r["all"]) and r["any"] for r in s)} none {sum(not r["any"] for r in s)} | split-ids all-gold {sum(r["all2"] for r in s)/len(s):.3f}')
miss = json.load(open(f'{ROOT}/pilot/misses_full.json'))['WARM wrong, Mem0 right']
cl = collections.Counter((m['cat'], 'all' if rows[m['id']]['all'] else 'partial' if rows[m['id']]['any'] else 'none') for m in miss)
print('classification of the 58 recomputed', dict(cl))
agree = sum(((rows[m['id']]['all'] and m['class'].startswith('all')) or ((not rows[m['id']]['all']) and rows[m['id']]['any'] and m['class'].startswith('some')) or ((not rows[m['id']]['any']) and m['class'].startswith('no gold'))) for m in miss)
print('agree with misses_full.json classes:', agree, 'of', len(miss))
json.dump(rows, open('phase0_rows.json', 'w'))
