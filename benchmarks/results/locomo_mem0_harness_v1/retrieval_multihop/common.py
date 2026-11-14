import json, glob, os, re, hashlib, sqlite3, subprocess, urllib.request, time, shutil, tempfile
import numpy as np
H = os.path.expanduser('~')
ROOT = '/Volumes/A/.hms-lb/locomo-20261114'
DATA = json.load(open(f'{ROOT}/locomo10.json'))
CACHE = f'{ROOT}/../memeval-20261112/locomo/cache.sqlite'
CATS = {1: 'multi-hop', 2: 'temporal', 3: 'open-domain', 4: 'single-hop'}

def turns_of(ci):
    """Ordered (dia_id, text-as-ingested) for a conversation, in the harness's chronological session order."""
    c = DATA[ci]['conversation']
    keys = sorted((k for k in c if re.fullmatch(r'session_\d+', k)), key=lambda k: int(k.split('_')[1]))
    out = []
    for k in keys:
        for t in c[k]:
            out.append((t['dia_id'], t['speaker'], t['text'], k))
    return out

def ingested_text(t):
    d = t
    return d

def load_questions():
    qs = []
    for ci, conv in enumerate(DATA):
        for qi, q in enumerate(conv['qa']):
            if q.get('category') == 5 or not q.get('evidence'):
                continue
            qs.append({'id': f'conv{ci}_q{qi}', 'ci': ci, 'cat': CATS[q['category']], 'q': q['question'], 'ev': q['evidence']})
    return qs

def stored():
    res = {}
    for f in glob.glob(H + '/pilot-out/all/predicted_all/conv*_q*.json'):
        e = json.load(open(f)); res[e['question_id']] = e
    return res

def digest(*parts):
    return hashlib.sha256('\x1f'.join(parts).encode()).hexdigest()

class CacheOnly:
    def __init__(self):
        self.db = sqlite3.connect(f'file:{CACHE}?mode=ro', uri=True)
    def embed(self, text):
        row = self.db.execute('SELECT v FROM kv WHERE k=?', (digest('embed', 'text-embedding-3-small', text),)).fetchone()
        if row is None:
            raise SystemExit('embedding cache miss; refusing to call the API: ' + text[:60])
        return json.loads(row[0])

def boot(ci, lo, hi, n=10000, seed=0):
    rng = np.random.default_rng(seed)
    return rng

HOST = 'http://127.0.0.1:18777'
RUN = 'a3a6075e'
def parse_date(s):
    from datetime import datetime, timezone
    for fmt in ("%I:%M %p on %d %B, %Y", "%I:%M %p on %d %b, %Y"):
        try: return int(datetime.strptime(s, fmt).replace(tzinfo=timezone.utc).timestamp())
        except ValueError: pass

def turn_table(ci):
    """Per-conversation: list of dicts in ingestion order with the exact fact id the harness client assigned."""
    c = DATA[ci]['conversation']
    from datetime import datetime
    sess = []
    for k in c:
        if re.fullmatch(r'session_\d+', k):
            d = c.get(k + '_date_time', '')
            p = None
            for fmt in ("%I:%M %p on %d %B, %Y", "%I:%M %p on %d %b, %Y"):
                try: p = datetime.strptime(d, fmt); break
                except ValueError: pass
            sess.append(((0, p) if p else (1, datetime(2000, 1, int(k.split('_')[1]))), k))
    sess.sort(key=lambda x: x[0])
    uid = f'locomo_{ci}_{RUN}'; out = []
    for _, k in sess:
        ts = parse_date(c.get(k + '_date_time', ''))
        for ti, t in enumerate(c[k]):
            text = t.get('text', ''); blip = t.get('blip_caption', ''); qy = t.get('query', '')
            tag = f'[Sharing image - query: {qy}. The image shows: {blip}]' if qy and blip else f'[Sharing image - query for: {qy}]' if qy else f'[Sharing image that shows: {blip}]' if blip else ''
            if tag: text = f'{text} {tag}' if text else tag
            if not text: continue
            content = f"{t['speaker']}: {text}"
            fid = 'w:' + hashlib.sha256(f'{uid}\x1f{ts}\x1f{content}'.encode()).hexdigest()[:24]
            out.append({'dia': t['dia_id'], 'sess': k, 'pos': len(out), 'ti': ti, 'speaker': t['speaker'], 'raw': t['text'], 'content': content, 'fid': fid, 'ts': ts})
    return out

def post(path, body, tenant, timeout=120):
    req = urllib.request.Request(f'{HOST}/api/v1/memory/{path}', data=json.dumps(body).encode(), headers={'Content-Type': 'application/json', 'X-Tenant-ID': tenant})
    with urllib.request.urlopen(req, timeout=timeout) as r: return json.loads(r.read())

def recall(ci, query, emb, k):
    t0 = time.perf_counter()
    r = post('recall', {'query': query, 'k': k, 'embedding': emb}, f'locomo_{ci}_{RUN}')
    return r['results'], (time.perf_counter() - t0) * 1000
