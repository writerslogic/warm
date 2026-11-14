# Cause analysis of lost gold turns (descriptive; no candidate method is measured here). Baseline = base200.json; ranks from deep1000.json (k=1000 server, approximate).
import json, collections, re
from common import *
qs = load_questions(); base = json.load(open('base200.json')); deep = json.load(open('deep1000.json'))
tt = {ci: turn_table(ci) for ci in range(10)}
by_dia = {ci: {t['dia']: t for t in tt[ci]} for ci in tt}; f2t = {ci: {t['fid']: t for t in tt[ci]} for ci in tt}
rows0 = json.load(open('phase0_rows.json'))
STOP = set('a an the of to in on at is are was were be been do does did what which who whom when where how why for and or with that this it its his her their they he she her from by as about after before'.split())
tok = lambda s: {w for w in re.findall(r"[a-z0-9]+", s.lower()) if w not in STOP}
ctr = collections.Counter(); detail = []
tot = collections.Counter()
for q in qs:
    if q['cat'] != 'multi-hop': continue
    ci = q['ci']; top = [i for i, _ in base[q['id']]]; topt = {f2t[ci][i]['pos'] for i in top}
    ev = rows0[q['id']]['ev2']; golds = [by_dia[ci][d] for d in ev if d in by_dia[ci]]
    present = [g['pos'] in topt for g in golds]
    tot['questions'] += 1
    if all(present): tot['all'] += 1; continue
    tot['partial_or_none'] += 1
    drank = {f2t[ci][i]['pos']: r + 1 for r, (i, _) in enumerate(deep[q['id']])}
    qt = tok(q['q'])
    for g, p in zip(golds, present):
        if p: continue
        words = len(g['raw'].split()); rk = drank.get(g['pos'])
        gt = tok(g['content'])
        nbr = [x for x in (g['pos'] - 1, g['pos'] + 1) if 0 <= x < len(tt[ci])]
        feats = {
            'q': q['id'], 'dia': g['dia'], 'rank1000': rk, 'words': words, 'has_image': '[Sharing image' in g['content'],
            'short<=8w': words <= 8, 'lex_overlap_with_question': len(qt & gt), 'neighbour_in_top200': any(n in topt for n in nbr),
            'same_session_gold_present': any(p2 and g2['sess'] == g['sess'] for g2, p2 in zip(golds, present)),
            'adjacent_gold_present': any(x['dia'] in ev and x['pos'] in topt for x in (tt[ci][n] for n in nbr)),
            'n_ev': len(golds), 'speaker_named_in_q': g['speaker'].lower() in q['q'].lower(), 'text': g['content'][:160]}
        detail.append(feats)
n = len(detail); print('multi-hop questions', tot['questions'], 'with all gold in top-200', tot['all'], 'missing >=1', tot['partial_or_none'], 'missing gold turns', n)
def pct(f): return f'{sum(f(d) for d in detail)} ({sum(f(d) for d in detail)/n:.0%})'
print('missing turns: in k=1000 list', pct(lambda d: d['rank1000'] is not None))
rks = sorted(d['rank1000'] for d in detail if d['rank1000'])
print('  rank in k=1000 quantiles (25/50/75/90):', [int(np.percentile(rks, p)) for p in (25, 50, 75, 90)], 'ranks 201-400:', sum(201 <= r <= 400 for r in rks), '401-1000:', sum(r > 400 for r in rks), 'absent from 1000:', n - len(rks))
print('short (<=8 words)', pct(lambda d: d['short<=8w'])); print('image-caption turn', pct(lambda d: d['has_image']))
print('zero lexical overlap with question (no stopwords)', pct(lambda d: d['lex_overlap_with_question'] == 0))
print('adjacent turn (+-1) already in top-200', pct(lambda d: d['neighbour_in_top200']))
print('adjacent turn is another gold turn present in top-200', pct(lambda d: d['adjacent_gold_present']))
print('same session as a gold turn already present', pct(lambda d: d['same_session_gold_present']))
print('evidence-set size of affected questions', dict(collections.Counter(len({d['dia'] for d in detail if d['q'] == q}) and next(x['n_ev'] for x in detail if x['q'] == q) for q in {d['q'] for d in detail})))
allw = [len(t['raw'].split()) for ci in tt for t in tt[ci]]; print('all turns: median words', np.median(allw), 'share <=8 words %.2f' % np.mean([w <= 8 for w in allw]), 'image share %.2f' % np.mean([('[Sharing image' in t['content']) for ci in tt for t in tt[ci]]))
gw = [len(by_dia[q['ci']][d]['raw'].split()) for q in qs if q['cat']=='multi-hop' for d in rows0[q['id']]['ev2'] if d in by_dia[q['ci']]]
print('all multi-hop gold turns: median words', np.median(gw), 'share short %.2f' % np.mean([w <= 8 for w in gw]))
# Per-question number of questions whose miss would be recovered by each (descriptive reachability, not a method result)
qm = collections.defaultdict(list)
for d in detail: qm[d['q']].append(d)
print('partial questions', len(qm), '| every missing turn has adjacent gold present:', sum(all(x['adjacent_gold_present'] for x in v) for v in qm.values()), '| every missing turn has neighbour in top200:', sum(all(x['neighbour_in_top200'] for x in v) for v in qm.values()), '| every missing turn within rank 400:', sum(all((x['rank1000'] or 9999) <= 400 for x in v) for v in qm.values()))
json.dump(detail, open('diag_missing_gold.json', 'w'), indent=1)
for d in detail[:6]: print(d['q'], d['dia'], d['rank1000'], d['text'])
