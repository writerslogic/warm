# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tool layer over the compiled memory (envmodel.py) for the memory-side controller. Deterministic, no LLM, no gold.
Compiled products: region units (R), transition effects (T), episodes (E), page inventories (P), per-step value log (V), trajectory notes (N)."""
import re, collections
import envmodel as EM
from envmodel import Model, Index, parse_tree, finish, value_lines, toks

class Model2(Model):
    """Model plus a per-step value log: vlog[tid] = [(state_index, {element label: value string})]."""
    def __init__(s):
        super().__init__(); s.vlog = {}; s.goals = {}
    def add_traj(s, t):
        super().add_traj(t)
        s.goals[t['id']] = t['goal']; log = []
        for st in t['states']:
            root, _ = parse_tree(st['accessibility_tree'] or '')
            if root is None: continue
            finish(root); d = {}
            for ln in value_lines(root):
                m = re.match(r"(.*?) ((?:value=|checked|disabled|selected|expanded|pressed|required|level).*)$", ln)
                if m: d[m.group(1)] = m.group(2)
            if d: log.append((st['state_index'], d))
        s.vlog[t['id']] = log

class Tools:
    def __init__(s, traj_ids, T, notes=None, k_max=8):
        s.m = Model2()
        for i in traj_ids: s.m.add_traj(T[i])
        s.ix = Index(s.m); s.notes = notes or {}
        s.nk = [k for k in s.notes if k in s.m.eps]
        s.nd = [s._note_text(k) for k in s.nk]
        s.n_bm = EM.BM25([toks(x) for x in s.nd]) if s.nd else None
        s.ptm = {i: k for i, k in enumerate(s.ix.tk)}
        s.vl = [(tid, si, el, v) for tid, lg in s.m.vlog.items() for si, d in lg for el, v in d.items()]
        s.v_bm = EM.BM25([toks(el + ' ' + v) for _, _, el, v in s.vl])
        s.k_max = k_max
    def _note_text(s, tid):
        n = s.notes[tid]; out = []
        for kind in ('procedure', 'hint'):
            x = n.get(kind) or {}
            out.append(f"{kind.upper()} {x.get('title','')}: {x.get('description','')}\n{x.get('content','')}")
        return f"Trajectory {tid} goal: {s.m.goals.get(tid,'')[:200]}\n" + '\n'.join(out)
    # ---- tool implementations; each returns text ----
    def search(s, kind, query, k=6):
        k = max(1, min(int(k or 6), s.k_max)); qt = toks(query); out = []
        if kind == 'regions':
            for i, _ in s.ix.u_bm.search(qt, k): out.append(f"R{i} | {s.ix.unit_text(s.ix.ud[i])[:380]}")
        elif kind == 'transitions':
            for i, _ in s.ix.e_bm.search(qt, k): out.append(f"T{i} | {s.ix.ed[i][:380]}")
        elif kind == 'episodes':
            for i, _ in s.ix.p_bm.search(qt, k): out.append(f"E:{s.ix.pk[i]} | {s.m.eps[s.ix.pk[i]][:380]}")
        elif kind == 'pages':
            for i, _ in s.ix.t_bm.search(qt, k):
                v = s.m.tmpl[s.ix.tk[i]]; out.append(f"P{i} | page '{s.ix.tk[i][1]}' [{s.ix.tk[i][0][:70]}] {v['n']} states, {len(v['inv'])} distinct elements")
        elif kind == 'notes':
            if s.n_bm:
                for i, _ in s.n_bm.search(qt, k): out.append(f"N:{s.nk[i]} | {s.nd[i][:380]}")
        else: return "unknown kind; use regions, transitions, episodes, pages, notes"
        return '\n'.join(out) or "no results"
    def open(s, id_):
        try:
            if id_.startswith('R'): return s.ix.unit_text(s.ix.ud[int(id_[1:])])[:4000]
            if id_.startswith('T'): return s.ix.ed[int(id_[1:])][:2000]
            if id_.startswith('E:'): return s.m.eps[id_[2:]][:5000]
            if id_.startswith('P'): return s.ix.tmpl_text(s.ix.tk[int(id_[1:])], maxc=6000)
            if id_.startswith('N:'): return s.nd[s.nk.index(id_[2:])][:4000]
        except Exception: pass
        return "unknown id"
    def transitions_from(s, page_id, k=12):
        try: tk = s.ix.tk[int(page_id[1:])]
        except Exception: return "unknown page id (use an id like P12 from a pages search)"
        rows = [(e['n'], i) for i, key in enumerate(s.ix.ek) if key[0] == tk for e in [s.m.eff[key]]]
        rows.sort(reverse=True)
        return '\n'.join(f"T{i} | {s.ix.ed[i][:420]}" for _, i in rows[:min(int(k or 12), 15)]) or "no recorded actions from this page"
    def value_history(s, query, k=5):
        qt = set(toks(query)); hits = [i for i, _ in s.v_bm.search(list(qt), 400)]
        seq = collections.defaultdict(list)
        for i in hits:
            tid, si, el, v = s.vl[i]
            seq[(tid, el)].append((int(si), v))
        out = []
        for (tid, el), xs in sorted(seq.items(), key=lambda kv: -len(kv[1]))[:min(int(k or 5), 8)]:
            xs.sort(); ch = []; last = None
            for si, v in xs:
                if v != last: ch.append(f"step {si}: {v[:80]}"); last = v
            out.append(f"{tid} {el[:70]}: " + ' -> '.join(ch[:8]))
        return '\n'.join(out) or "no values logged for that"
    def dispatch(s, name, a):
        try:
            if name == 'search': r = s.search(a.get('kind', 'regions'), a.get('query', ''), a.get('k', 6))
            elif name == 'open': r = s.open(str(a.get('id', '')))
            elif name == 'transitions_from': r = s.transitions_from(str(a.get('page_id', '')))
            elif name == 'value_history': r = s.value_history(a.get('query', ''))
            else: r = 'unknown tool'
        except Exception as e: r = f"tool error: {e!r}"[:200]
        return r[:6500]

