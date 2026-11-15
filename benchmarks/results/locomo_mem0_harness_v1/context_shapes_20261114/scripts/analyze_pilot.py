import json, glob, sys, collections, os
R = '/Users/davidcondrey/warm-locomo2/benchmarks/results/locomo_mem0_harness_v1/per_question.jsonl'
base = {json.loads(l)['question_id']: json.loads(l) for l in open(R)}
Q = json.load(open('pilot_questions.json')); ids = Q['ids']
W = os.path.expanduser('~/ctx3-out')
def load(phase):
    out = {}
    for f in glob.glob(f'{W}/{phase}/predicted_{phase}/conv*_q*.json'):
        e = json.load(open(f)); out[e['question_id']] = e
    return out
def usage(phase):
    u = [json.loads(l) for l in open(f'{phase}-usage.jsonl')]; a = [x for x in u if x['kind'] == 'answer']
    return {'usd': sum(x['usd'] for x in u), 'answer_calls': len(a), 'length': sum(x['finish'] == 'length' for x in a), 'mean_prompt': sum(x['prompt'] for x in a) / len(a), 'mean_reasoning': sum(x['reasoning'] for x in a) / len(a)}
cats = ('multi-hop', 'temporal', 'open-domain', 'single-hop')
res = {}
for ph in sys.argv[1:]:
    r = load(ph); est = 0; line = {}
    for c in cats:
        F = [i for i in ids if base[i]['category'] == c and base[i]['warm_judgment'] != 'CORRECT']
        S = [i for i in ids if base[i]['category'] == c and base[i]['warm_judgment'] == 'CORRECT']
        full = Q['info'][c]
        fix = sum(r[i]['cutoff_results']['top_200']['judgment'] == 'CORRECT' for i in F if i in r) / max(1, len(F))
        keep = sum(r[i]['cutoff_results']['top_200']['judgment'] == 'CORRECT' for i in S if i in r) / max(1, len(S))
        est += full['dev_fail'] * fix + full['dev_ok'] * keep
        empt = sum(r[i]['cutoff_results']['top_200']['generated_answer'].strip() == '' for i in F + S if i in r)
        line[c] = {'fail_n': len(F), 'fixed': round(fix, 3), 'succ_n': len(S), 'kept': round(keep, 3), 'empty': empt}
    res[ph] = {'est_dev_acc': est / Q['n_dev'], 'cats': line, 'usage': usage(ph), 'answered': len(r)}
    print(ph, json.dumps(res[ph], indent=1))
print('baseline dev acc', 1 - sum(1 for i in base if int(i.split('_')[0][4:]) <= 4 and base[i]['warm_judgment'] != 'CORRECT') / Q['n_dev'])
json.dump(res, open('pilot_analysis.json', 'w'), indent=1)
