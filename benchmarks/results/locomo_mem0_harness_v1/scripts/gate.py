import json, os, sys, glob
OUT, P = sys.argv[1], '/Volumes/A/.hms-lb/locomo-20261114/pilot'
H = os.path.expanduser('~')
os.environ['OPENAI_API_KEY'] = open(H + '/.config/warm-bench/lme2-openai.key').read().strip()
os.environ['WARM_EMBED_BASE_URL'] = 'https://api.openai.com/v1'
sys.path.insert(0, H + '/.hms-src-locomo/benchmarks/public/locomo')
import mem0_harness_warm as w
data = json.load(open('/Volumes/A/.hms-lb/locomo-20261114/locomo10.json'))
counts = {}
for l in open(H + '/pilot-out/all-server.log', errors='replace'):
    if '/memory/remember' not in l: continue
    try: r = json.loads(l)
    except Exception: continue
    if r.get('status') == 200: counts[r['tenant']] = counts.get(r['tenant'], 0) + 1
c = w.WarmClient()
ok = True; lines = []
for i, conv in enumerate(data):
    ck = json.load(open(f'{OUT}/predicted_all/_ingestion_{i}.json')); uid = ck['user_id']
    turns = [t for k, v in conv['conversation'].items() if isinstance(v, list) for t in v]
    n, got, proc = len(turns), counts.get(uid, 0), ck['total_chunks_processed']
    t = turns[len(turns) // 2]; text = t['text']
    emb = c._embed([text])[0]
    hits = c._memory(uid, 'recall', {'query': text, 'k': 5, 'embedding': emb})['results']
    spot = any(text in h['text'] for h in hits)
    good = (n == got == proc) and ck['total_chunks_failed'] == 0 and spot
    ok &= good
    lines.append(f'conv {i} tenant {uid} dataset_turns={n} chunks_processed={proc} server_remember_200={got} failed={ck["total_chunks_failed"]} spot_recall({t["dia_id"]})={"PASS" if spot else "FAIL"} -> {"OK" if good else "MISMATCH"}')
lines.append('GATE ' + ('PASSED' if ok else 'FAILED'))
open(P + '/ingest-gate.txt', 'w').write('\n'.join(lines) + '\n')
sys.exit(0 if ok else 4)
