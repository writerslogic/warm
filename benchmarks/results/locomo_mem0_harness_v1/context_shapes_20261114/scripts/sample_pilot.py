import json, random, collections
R = '/Users/davidcondrey/warm-locomo2/benchmarks/results/locomo_mem0_harness_v1/per_question.jsonl'
rows = [json.loads(l) for l in open(R)]
dev = [r for r in rows if int(r['question_id'].split('_')[0][4:]) <= 4]
fails = [r for r in dev if r['warm_judgment'] != 'CORRECT']
oks = [r for r in dev if r['warm_judgment'] == 'CORRECT']
rng = random.Random(0); chosen = [r['question_id'] for r in fails]; info = {}
for c in ('multi-hop', 'temporal', 'open-domain', 'single-hop'):
    f = [r for r in fails if r['category'] == c]; o = sorted((r for r in oks if r['category'] == c), key=lambda r: r['question_id'])
    n = min(len(o), max(len(f), 8)); pick = rng.sample(o, n); chosen += [r['question_id'] for r in pick]
    info[c] = {'dev_fail': len(f), 'dev_ok': len(o), 'sampled_ok': n}
json.dump({'ids': chosen, 'info': info, 'n_dev': len(dev)}, open('pilot_questions.json', 'w'), indent=1)
print(info, len(chosen), 'dev n', len(dev))
