# Assemble the committed evidence directory from the verified pilot outputs. No network, no key.
import json, glob, os, hashlib, shutil, sys
H = os.path.expanduser('~'); R = '/Volumes/A/.hms-lb/locomo-20261114'; P = R + '/pilot'
D = H + '/warm-locomo2/benchmarks/results/locomo_mem0_harness_v1'
os.makedirs(D + '/scripts', exist_ok=True); os.makedirs(D + '/mem0_published', exist_ok=True)
for f in ('full1540-analysis.txt', 'ingest-gate.txt', 'run.py.diff', 'misses_full.json', 'all-usage.jsonl', 'all-child-env-names.txt'):
    shutil.copy(f'{P}/{f}', f'{D}/{f}')
for f in ('analyze_full.py', 'run_full.py', 'gate.py', 'run_pilot.py'):
    shutil.copy(f'{P}/{f}', f'{D}/scripts/{f}')
M0 = R + '/mem0-benchmarks/f/results/platform/locomo_results.json'
shutil.copy(M0, D + '/mem0_published/locomo_results.json'); shutil.copy(R + '/mem0-benchmarks/repo/LICENSE', D + '/mem0_published/LICENSE-memory-benchmarks-Apache-2.0')
m0 = {e['question_id']: e for e in json.load(open(M0))['evaluations']}
data = json.load(open(R + '/locomo10.json'))
turn = {(ci, t['dia_id']): t['text'] for ci, c in enumerate(data) for k, v in c['conversation'].items() if isinstance(v, list) for t in v}
n = 0; empty = 0
with open(D + '/per_question.jsonl', 'w') as out:
    for f in sorted(glob.glob(H + '/pilot-out/all/predicted_all/conv*_q*.json'), key=lambda p: (int(os.path.basename(p).split('_')[0][4:]), int(os.path.basename(p).split('_q')[1][:-5]))):
        e = json.load(open(f)); r = e['retrieval']; mem = [x['memory'] for x in r['search_results']]; ids = [x['id'] for x in r['search_results']]
        ev = e['evidence']; ci = int(e['conversation_idx'])
        pres = [any(turn.get((ci, d), '\0') in x for x in mem) for d in ev]
        cls = 'no evidence listed' if not ev else 'all gold present' if all(pres) else 'some gold absent' if any(pres) else 'no gold present'
        w = e['cutoff_results']['top_200']; m = m0[e['question_id']]['cutoff_results']['top_200']
        empty += not (w.get('generated_answer') or '').strip()
        out.write(json.dumps({'question_id': e['question_id'], 'category': e['category_name'], 'question': e['question'], 'gold_answer': e['ground_truth_answer'], 'evidence': ev,
            'warm_answer': w.get('generated_answer'), 'warm_judgment': w.get('judgment'), 'warm_score': w.get('score'), 'warm_judge_reason': w.get('reason'),
            'memories_evaluated': w.get('memories_evaluated'), 'search_latency_ms': r.get('search_latency_ms'), 'retrieved_ids_sha256': hashlib.sha256('\n'.join(ids).encode()).hexdigest(),
            'gold_evidence_in_top200': cls, 'mem0_score': m.get('score')}, ensure_ascii=False) + '\n'); n += 1
print('per_question rows', n, 'empty warm answers', empty)
print('mem0 file sha256', hashlib.sha256(open(M0, 'rb').read()).hexdigest())
