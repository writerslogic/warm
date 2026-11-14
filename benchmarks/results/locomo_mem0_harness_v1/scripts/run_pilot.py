"""Pilot driver: WARM behind Mem0's unmodified harness against the real OpenAI API.
Key: read from the key file into this process's environment only; never printed. Spend guard, usage log and a call cache
(so calibration answers are reused) wrap the OpenAI SDK; the first quota or billing error stops the process."""
import hashlib, json, os, re, subprocess, sys, threading, time, runpy
P = '/Volumes/A/.hms-lb/locomo-20261114/pilot'
phase, cap, dataset, outdir, extra = sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5:]
os.environ['OPENAI_API_KEY'] = open(os.path.expanduser('~/.config/warm-bench/lme2-openai.key')).read().strip()
names = sorted(os.environ)
open(f'{P}/{phase}-child-env-names.txt', 'w').write('\n'.join(names) + '\n')
bad = [n for n in names if re.search(r'(KEY|TOKEN|SECRET|PASSWORD)', n) and n != 'OPENAI_API_KEY']
assert not bad, bad
os.environ.update({'WARM_EMBED_BASE_URL': 'https://api.openai.com/v1', 'WARM_EMBED_DIM': '1536', 'WARM_MAX_TOP_K': '200',
                   'WARM_SERVER_BIN': '/Volumes/C/.hms-tgt/locomo/release/pilot-memsrv'})
PRICE = {'in': 1.25, 'cached': 0.125, 'out': 10.0}
lock = threading.Lock(); spent = [0.0]; CACHE = f'{P}/call-cache.jsonl'; USAGE = f'{P}/{phase}-usage.jsonl'
cache = {}
if os.path.exists(CACHE):
    for l in open(CACHE): r = json.loads(l); cache[r['k']] = r['v']
def die(msg):
    open(f'{P}/{phase}-STOPPED.txt', 'w').write(msg + '\n'); print('STOP', msg, file=sys.stderr, flush=True); os._exit(3)
real_popen = subprocess.Popen
def popen(*a, **k):
    env = dict(k.get('env') or os.environ); env.pop('OPENAI_API_KEY', None); k['env'] = env
    return real_popen(*a, **k)
subprocess.Popen = popen
import openai
from openai.resources.chat.completions import AsyncCompletions
orig = AsyncCompletions.create
async def create(self, **kw):
    key = hashlib.sha256(json.dumps({k: kw[k] for k in ('model', 'messages', 'response_format') if k in kw}, sort_keys=True, default=str).encode()).hexdigest()
    if key in cache:
        from openai.types.chat import ChatCompletion
        return ChatCompletion.model_validate_json(cache[key])
    if spent[0] >= cap: die(f'spend cap {cap} reached at {spent[0]:.4f}')
    try:
        resp = await orig(self, **kw)
    except openai.RateLimitError as e:
        if 'insufficient_quota' in str(e) or 'billing' in str(e).lower(): die('quota or billing error: ' + type(e).__name__)
        raise
    except openai.APIStatusError as e:
        if 'insufficient_quota' in str(e) or 'billing' in str(e).lower(): die('quota or billing error: ' + type(e).__name__)
        raise
    u = resp.usage
    det = getattr(u, 'prompt_tokens_details', None); cached = getattr(det, 'cached_tokens', 0) or 0
    rd = getattr(getattr(u, 'completion_tokens_details', None), 'reasoning_tokens', 0) or 0
    cost = ((u.prompt_tokens - cached) * PRICE['in'] + cached * PRICE['cached'] + u.completion_tokens * PRICE['out']) / 1e6
    kind = 'judge' if 'response_format' in kw else 'answer'
    with lock:
        spent[0] += cost
        open(USAGE, 'a').write(json.dumps({'kind': kind, 'model_returned': resp.model, 'model_requested': kw.get('model'), 'prompt': u.prompt_tokens, 'cached': cached,
                                           'completion': u.completion_tokens, 'reasoning': rd, 'finish': resp.choices[0].finish_reason, 'usd': round(cost, 6), 'key': key}) + '\n')
        cache[key] = resp.model_dump_json(); open(CACHE, 'a').write(json.dumps({'k': key, 'v': cache[key]}) + '\n')
    return resp
AsyncCompletions.create = create
sys.path.insert(0, '/Users/davidcondrey/.hms-src-locomo/benchmarks/public/locomo')
sys.argv = ['run_mem0_harness.py', '--harness', '/Volumes/A/.hms-lb/locomo-20261114/mem0-benchmarks/repo', '--benchmark', 'locomo', '--', '--project-name', phase,
            '--dataset-path', dataset, '--conversations', os.environ.get('PILOT_CONVS', '0,1,2'), '--top-k', '200', '--top-k-cutoffs', '200', '--output-dir', outdir,
            '--max-workers', os.environ.get('PILOT_WORKERS', '4'), '--answerer-model', 'gpt-5', '--judge-model', 'gpt-5', '--provider', 'openai'] + extra
runpy.run_path('/Users/davidcondrey/.hms-src-locomo/benchmarks/public/locomo/run_mem0_harness.py', run_name='__main__')
print('SPENT_LLM_USD', round(spent[0], 4), flush=True)
