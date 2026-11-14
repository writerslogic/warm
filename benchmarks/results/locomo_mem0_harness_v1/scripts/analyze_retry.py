import json, glob, numpy as np, os
P = '/Volumes/A/.hms-lb/locomo-20261114/pilot'; H = os.path.expanduser('~')
res = {}
for f in glob.glob(H + '/pilot-out/all/predicted_all/conv*_q*.json'):
    e = json.load(open(f)); res[e['question_id']] = (e['category_name'], e['cutoff_results']['top_200']['score'])
m0 = {e['question_id']: e['cutoff_results']['top_200']['score'] for e in json.load(open('/Volumes/A/.hms-lb/locomo-20261114/mem0-benchmarks/f/results/platform/locomo_results.json'))['evaluations']}
rt = {x['question_id']: x['score'] for x in json.load(open(P + '/retry-results.json'))}
ids = sorted(res); rng = np.random.default_rng(0)
def ci(d):
    m = d[rng.integers(0, len(d), (10000, len(d)))].mean(1); return d.mean(), np.percentile(m, 2.5), np.percentile(m, 97.5)
for label, adj in (('ORIGINAL SINGLE PASS', {}), ('WITH ONE IDENTICAL-SETTINGS RETRY OF EMPTY RESPONSES', rt)):
    print(label)
    for name in (None, 'multi-hop', 'temporal', 'open-domain', 'single-hop'):
        s = [i for i in ids if name is None or res[i][0] == name]
        w = np.array([adj.get(i, res[i][1]) for i in s]); m = np.array([m0[i] for i in s]); d = ci(w - m)
        print(f'  {name or "overall":12s} n={len(s):4d} WARM {w.mean()*100:5.1f}% ({int(w.sum())}) Mem0 {m.mean()*100:5.1f}% ({int(m.sum())}) diff {d[0]*100:+5.1f} pp [{d[1]*100:+5.1f}, {d[2]*100:+5.1f}]')
