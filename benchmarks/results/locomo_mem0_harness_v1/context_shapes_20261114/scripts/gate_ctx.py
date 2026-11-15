import json, os, sys, glob
OUT, SERVERLOG, CONVS, GATEOUT = sys.argv[1], sys.argv[2], [int(x) for x in sys.argv[3].split(',')], sys.argv[4]
H = os.path.expanduser('~')
os.environ['OPENAI_API_KEY'] = open(H + '/.config/warm-bench/lme2-openai.key').read().strip()
os.environ['WARM_EMBED_BASE_URL'] = 'https://api.openai.com/v1'
sys.path.insert(0, H + '/warm-locomo2/benchmarks/public/locomo')
import mem0_harness_warm as w
data = json.load(open('/Volumes/A/.hms-lb/locomo-20261114/locomo10.json'))
counts = {}
for l in open(SERVERLOG, errors='replace'):
    if '/memory/remember' not in l: continue
    try: r = json.loads(l)
    except Exception: continue
    if r.get('status') == 200: counts[r['tenant']] = counts.get(r['tenant'], 0) + 1
sys.path.insert(0, '/Volumes/A/.hms-lb/locomo-20261114/multihop2')
import common
INDEX_DIR = os.path.dirname(OUT.rstrip('/')) + '/' + os.path.basename(OUT) + '-index'
c = w.WarmClient()
ok = True; lines = []
for i in CONVS:
    conv = data[i]
    ck = json.load(open(f'{OUT}/predicted_{os.path.basename(OUT)}/_ingestion_{i}.json')); uid = ck['user_id']
    turns = [t for k, v in conv['conversation'].items() if isinstance(v, list) for t in v]
    n, got, proc = len(turns), counts.get(uid, 0), ck['total_chunks_processed']
    t = turns[len(turns) // 2]; text = t['text']
    emb = c._embed([text])[0]
    hits = c._memory(uid, 'recall', {'query': text, 'k': 5, 'embedding': emb})['results']
    spot = any(text in h['text'] for h in hits)
    tt = common.turn_table(i)
    live = [json.loads(l) for l in open(f'{INDEX_DIR}/{uid}.jsonl')]
    order_ok = [(x[1], x[2]) for x in live] == [(r['content'], r['ts']) for r in tt]
    dates = [conv['conversation'][k + '_date_time'] for k in conv['conversation'] if k.startswith('session_') and k + '_date_time' in conv['conversation']]
    shared = len(dates) - len(set(dates))
    lines.append(f'conv {i} turn order and epochs equal the dataset: {order_ok}; sessions sharing a date_time: {shared}')
    ok &= order_ok and shared == 0
    good = (n == got == proc) and ck['total_chunks_failed'] == 0 and spot
    ok &= good
    lines.append(f'conv {i} tenant {uid} dataset_turns={n} chunks_processed={proc} server_remember_200={got} failed={ck["total_chunks_failed"]} spot_recall({t["dia_id"]})={"PASS" if spot else "FAIL"} -> {"OK" if good else "MISMATCH"}')
lines.append('GATE ' + ('PASSED' if ok else 'FAILED'))
open(GATEOUT, 'w').write('\n'.join(lines) + '\n')
sys.exit(0 if ok else 4)
