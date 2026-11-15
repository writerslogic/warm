import json, glob, os, sys
W = os.path.expanduser('~/ctx3-out')
dest = sys.argv[1]; os.makedirs(dest, exist_ok=True)
for ph in ('pilotO', 'pilotM', 'confA'):
    rows = []
    for f in sorted(glob.glob(f'{W}/{ph}/predicted_{ph}/conv*_q*.json')):
        e = json.load(open(f)); c = e['cutoff_results']['top_200']
        rows.append({'question_id': e['question_id'], 'category': e['category_name'], 'question': e['question'], 'gold_answer': e['ground_truth_answer'], 'evidence': e.get('evidence'),
                     'answer': c['generated_answer'], 'judgment': c['judgment'], 'score': c['score'], 'reason': c.get('reason', ''), 'items': c['memories_evaluated']})
    with open(f'{dest}/per_question_{ph}.jsonl', 'w') as o:
        for r in rows: o.write(json.dumps(r) + '\n')
    print(ph, len(rows))
