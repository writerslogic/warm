"""analyze_final.py DIR [DIR...]: paired bootstrap of the new WARM run (results in each DIR/predicted_*/conv*_q*.json) against Mem0's published per-question file and against the earlier WARM run, on the questions present."""
import json, glob, sys, os, numpy as np
B = '$WARM_REPO/benchmarks/results/locomo_mem0_harness_v1'
m0 = {e['question_id']: e for e in json.load(open(f'{B}/mem0_published/locomo_results.json'))['evaluations']}
old = {json.loads(l)['question_id']: json.loads(l) for l in open(f'{B}/per_question.jsonl')}
res = {}
for d in sys.argv[1:]:
    for f in glob.glob(f'{d}/predicted_*/conv*_q*.json'):
        e = json.load(open(f)); res[e['question_id']] = e
S = lambda e: e['cutoff_results']['top_200']['score']
ids = sorted(i for i in res if i in m0)
rng = np.random.default_rng(0)
def ci_(d):
    m = d[rng.integers(0, len(d), (10000, len(d)))].mean(1); return d.mean(), np.percentile(m, 2.5), np.percentile(m, 97.5)
out = {}
def line(name, sub):
    w = np.array([S(res[i]) for i in sub]); m = np.array([S(m0[i]) for i in sub]); o = np.array([old[i]['warm_score'] for i in sub])
    dm, do, dom = ci_(w - m), ci_(w - o), ci_(o - m)
    verdict = 'ahead' if dm[1] > 0 else 'behind' if dm[2] < 0 else ('tied, point estimate ahead' if dm[0] > 0 else 'tied, point estimate behind' if dm[0] < 0 else 'tied')
    out[name] = {'n': len(sub), 'warm_new': w.mean(), 'warm_old': o.mean(), 'mem0': m.mean(), 'diff_vs_mem0': dm, 'diff_vs_old': do, 'verdict': verdict, 'old_vs_mem0': dom}
    print(f'{name:12s} n={len(sub):4d} new {w.mean()*100:5.1f}% old {o.mean()*100:5.1f}% Mem0 {m.mean()*100:5.1f}% | vs Mem0 {dm[0]*100:+5.1f} pp [{dm[1]*100:+5.1f}, {dm[2]*100:+5.1f}] {verdict} | vs old {do[0]*100:+5.1f} [{do[1]*100:+5.1f}, {do[2]*100:+5.1f}] | old vs Mem0 {dom[0]*100:+5.1f} [{dom[1]*100:+5.1f}, {dom[2]*100:+5.1f}]')
print('questions', len(ids), 'of 1540; conversations', sorted({res[i]['conversation_idx'] for i in ids}))
line('overall', ids)
for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'):
    s = [i for i in ids if res[i]['category_name'] == c]
    if s: line(c, s)
emp = [i for i in ids if res[i]['cutoff_results']['top_200']['generated_answer'].strip() == '']
print('empty answers new:', len(emp), ' old on same questions:', sum(old[i]['warm_answer'].strip() == '' for i in ids), ' Mem0:', sum(m0[i]['cutoff_results']['top_200']['generated_answer'].strip() == '' for i in ids))
json.dump({'rows': out, 'empty_new': len(emp), 'empty_ids': emp}, open('final_analysis.json', 'w'), indent=1)
