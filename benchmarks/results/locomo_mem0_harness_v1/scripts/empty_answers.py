# Post hoc, descriptive: questions where WARM's reader returned no answer (completion cap hit by reasoning). Run in the result directory.
import json, collections, numpy as np
rows = [json.loads(l) for l in open('per_question.jsonl')]
em = {r['question_id'] for r in rows if not (r['warm_answer'] or '').strip()}
m0 = json.load(open('mem0_published/locomo_results.json'))['evaluations']
print('WARM empty answers', len(em), 'by category', dict(collections.Counter(r['category'] for r in rows if r['question_id'] in em)))
print('Mem0 empty answers', sum(not (e['cutoff_results']['top_200'].get('generated_answer') or '').strip() for e in m0))
print('of the WARM-empty questions Mem0 answered correctly', sum(r['mem0_score'] >= .5 for r in rows if r['question_id'] in em))
print('empty answers by gold-evidence class', dict(collections.Counter(r['gold_evidence_in_top200'] for r in rows if r['question_id'] in em)))
miss = json.load(open('misses_full.json'))['WARM wrong, Mem0 right']; mm = [m for m in miss if m['cat'] == 'multi-hop']
print('WARM wrong, Mem0 right:', len(miss), 'of which empty', sum(m['id'] in em for m in miss), '| multi-hop', len(mm), 'of which empty', sum(m['id'] in em for m in mm),
      '| partial-retrieval-miss class', sum(m['class'].startswith('some') for m in miss), 'of which empty', sum(m['id'] in em for m in miss if m['class'].startswith('some')))
rng = np.random.default_rng(0)
def ci(d):
    d = np.array(d); m = d[rng.integers(0, len(d), (10000, len(d)))].mean(1); return d.mean() * 100, np.percentile(m, 2.5) * 100, np.percentile(m, 97.5) * 100
for name, sel in (('overall', lambda r: True), ('multi-hop', lambda r: r['category'] == 'multi-hop')):
    s = [r for r in rows if sel(r) and r['question_id'] not in em]
    print(f'{name} excluding WARM-empty questions (post hoc, not a headline): n={len(s)} WARM {np.mean([r["warm_score"] for r in s])*100:.1f} Mem0 {np.mean([r["mem0_score"] for r in s])*100:.1f} diff {ci([r["warm_score"]-r["mem0_score"] for r in s])[0]:+.1f} [{ci([r["warm_score"]-r["mem0_score"] for r in s])[1]:+.1f}, {ci([r["warm_score"]-r["mem0_score"] for r in s])[2]:+.1f}]')
