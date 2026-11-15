"""One identical-settings retry of the 22 empty gpt-5 answers. Reuses run_pilot's key handling, spend cap, usage log and cache patch."""
import sys, os, json, asyncio, hashlib
P = '/Volumes/A/.hms-lb/locomo-20261114/pilot'; H = os.path.expanduser('~')
sys.argv = ['x', 'retry', '1.5', '/Volumes/A/.hms-lb/locomo-20261114/locomo10.json', '/nonexistent']
src = open(P + '/run_pilot.py').read().split("sys.path.insert(0, '/Users")[0]
exec(src)
REPO = '/Volumes/A/.hms-lb/locomo-20261114/mem0-benchmarks/repo'; sys.path.insert(0, REPO)
import benchmarks.locomo.run as R
from benchmarks.common.llm_client import LLMClient
data = json.load(open(sys.argv[3]))
ids = json.load(open(P + '/empty_answer_question_ids_local.json'))
orig_keys = {json.loads(l)['key'] for l in open(P + '/all-usage.jsonl') if json.loads(l)['kind'] == 'answer' and json.loads(l)['finish'] == 'length'}
answerer = LLMClient(model='gpt-5', provider='openai', rpm=200); judge = LLMClient(model='gpt-5', provider='openai', rpm=200)
def key_of(prompt):
    return hashlib.sha256(json.dumps({'model': 'gpt-5', 'messages': [{'role': 'user', 'content': prompt}]}, sort_keys=True, default=str).encode()).hexdigest()
async def main():
    out = []; plan = []
    for qid in ids:
        e = json.load(open(f'{H}/pilot-out/all/predicted_all/{qid}.json')); ci = e['conversation_idx']; qi = int(qid.split('_q')[1])
        qa = data[ci]['qa'][qi]; sliced = e['retrieval']['search_results'][:200]
        prompt = R.get_answer_generation_prompt(qa['question'], sliced, reference_date=e['reference_date'], user_profile=None)
        k = key_of(prompt); assert k in orig_keys, f'prompt not byte-identical for {qid}'
        # free check that the judge prompt rebuild reproduces the original judgment (cache hit)
        cat = qa['category']; pa = R.preprocess_answer(cat, str(qa['answer']))
        jp = R.get_judge_prompt(cat, qa['question'], pa, '')
        plan.append((qid, e, qa, prompt, k, cat, pa))
    print('all 22 answer prompts byte-identical to the originals', flush=True)
    for qid, e, qa, prompt, k, cat, pa in plan:
        cache.pop(k, None)  # bypass the cache for this key only
        g = await answerer.generate(system='', user=prompt)
        if 'ANSWER:' in g: g = g.rsplit('ANSWER:', 1)[-1].strip()
        raw = await judge.generate_structured(system=R.JUDGE_SYSTEM_PROMPT, user=R.get_judge_prompt(cat, qa['question'], pa, g))
        ok = isinstance(raw, dict) and raw.get('label', '').upper() == 'CORRECT'
        out.append({'question_id': qid, 'category_name': e['category_name'], 'new_answer': g, 'nonempty': bool(g.strip()), 'judgment': 'CORRECT' if ok else 'WRONG', 'score': 1.0 if ok else 0.0, 'reason': raw.get('reasoning', '') if isinstance(raw, dict) else ''})
        json.dump(out, open(P + '/retry-results.json', 'w'), indent=1)
asyncio.run(main())
print('SPENT_LLM_USD', round(spent[0], 4), flush=True)
