# Offline retrieval and prompt-size metrics for the context shapes (zero spend; cached query embeddings; no provider key).
import sys, os, json, importlib.util, time
MH = '$WARM_BENCH_DATA/multihop2'
sys.path.insert(0, MH); sys.path.insert(0, os.path.expanduser('~/warm-locomo2/benchmarks/public/locomo'))
os.chdir(MH)
from exp_lib import *            # qs, tt, by_dia, f2t, rows0, gold_pos, metrics, boot_ci, base ...
import context_shapes as cs
import tiktoken
enc = tiktoken.get_encoding('o200k_base')
spec = importlib.util.spec_from_file_location('prompts', '$WARM_BENCH_DATA/mem0-benchmarks/repo/benchmarks/locomo/prompts.py')
prompts = importlib.util.module_from_spec(spec); spec.loader.exec_module(prompts)
OUT = '$WARM_BENCH_DATA/context3'
ce = CacheOnly()
idx = {}
for ci in range(10):
    t = TI = cs.TenantIndex()
    for r in tt[ci]: TI.add(r['fid'], r['content'], r['ts'])
    idx[ci] = TI
    assert [r['fid'] for r in tt[ci]] == TI.ids and all(a == b['sess'] for a, b in zip([None]*0, []))
cache_f = f'{OUT}/hits400.json'
if os.path.exists(cache_f): H = json.load(open(cache_f))
else:
    H = {}
    for q in qs:
        e = ce.embed(q['q'])
        H[q['id']] = {'k200': [(h['id'], h['score']) for h in recall(q['ci'], q['q'], e, 200)[0]],
                      'k400': [(h['id'], h['score']) for h in recall(q['ci'], q['q'], e, 400)[0]]}
    json.dump(H, open(cache_f, 'w'))
def hit_dicts(ci, pairs): return [{'id': i, 'text': idx[ci].texts[idx[ci].seq[i]], 'score': s} for i, s in pairs]
def prompt_tokens(q, items):
    return len(enc.encode(prompts.get_answer_generation_prompt(q['q'], items, reference_date='May 7, 2023')))
CFGS = {'base': None, 'O': None}
for g in (0, 1):
    for T in (250, 300, 400): CFGS[f'M_g{g}_T{T}'] = (g, T)
res = {}
for name, par in CFGS.items():
    res[name] = {}
    for q in qs:
        ci = q['ci']; I = idx[ci]
        if name == 'base': hits = hit_dicts(ci, H[q['id']]['k200']); items = cs.ordered_items(hits, I, 200); pos = [I.seq[h['id']] for h in hits]
        elif name == 'O': hits = hit_dicts(ci, H[q['id']]['k200']); items = cs.ordered_items(hits, I, 200); pos = [I.seq[h['id']] for h in hits]
        else:
            hits = hit_dicts(ci, H[q['id']]['k400']); items = cs.merged_items(hits, I, 200, par[1], par[0])
            runs = cs.select_runs(hits, I, 200, par[1], par[0]); pos = [p for r in runs for p in r]
        if name == 'base':   # the baseline prompt: items in score order within a session (created_at without the index offset)
            from datetime import datetime, timezone
            for it in items: it['created_at'] = datetime.fromtimestamp(I.epochs[I.seq[it['id']]], tz=timezone.utc).isoformat()
        m = metrics(q, pos); m.update(items=len(items), turns=len(pos), tok=prompt_tokens(q, items)); res[name][q['id']] = m
json.dump(res, open(f'{OUT}/offline.json', 'w'))
cats = ('multi-hop', 'temporal', 'open-domain', 'single-hop')
def line(name, sel, label):
    ids = [q['id'] for q in qs if sel(q)]
    out = f'{name:12s} {label:5s}'
    for c in cats + ('ALL',):
        ii = [q['id'] for q in qs if sel(q) and (c == 'ALL' or q['cat'] == c)]
        out += f' {c[:5]} {np.mean([res[name][i]["all"] for i in ii]):.3f}'
    ii = ids
    out += f' | items {np.mean([res[name][i]["items"] for i in ii]):.0f} turns {np.mean([res[name][i]["turns"] for i in ii]):.0f} tok {np.mean([res[name][i]["tok"] for i in ii]):.0f}'
    return out
txt = []
for lab, sel in (('dev', lambda q: q['ci'] <= 4), ('held', lambda q: q['ci'] >= 5), ('all', lambda q: True)):
    for name in CFGS: txt.append(line(name, sel, lab))
    txt.append('')
open(f'{OUT}/offline.txt', 'w').write('\n'.join(txt)); print('\n'.join(txt))
