# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Write-time environment model for LongMemEval-V2 trajectories (deterministic, no LLM, no gold).
Parts: hash-consed AXTree region units (C1), transition effect table / automaton (C2), episodes (C3), value variants (C4)."""
import re, math, collections, ast, hashlib
from urllib.parse import unquote

LINE = re.compile(r"^(\t*)(?:\[(\d+)\]\s+)?(\S+)(?: '((?:[^'\\]|\\.)*)')?(.*)$")
KEEP = re.compile(r"(value='(?:[^'\\]|\\.)*'|\bchecked(?:=\w+)?\b|\bdisabled\b|\brequired\b|\bselected\b|\bexpanded=\w+|\bpressed=\w+|\blevel=\d+)")
SKIP_ROLES = {'generic', 'list', 'listitem', 'none', 'LabelText', 'group', 'presentation', 'separator', 'paragraph', 'Section', 'DescriptionList', 'rowgroup', 'cell_group'}
UNIT_MAX = 1500

def label(role, name):
    return f"{role} '{name[:80]}'" if name else role

class Node:
    __slots__ = ('role', 'name', 'pk', 'ch', 'h', 'c', 'size', 'bid')
    def __init__(s, role, name, pk, bid):
        s.role, s.name, s.pk, s.ch, s.bid = role, name, pk, [], bid

def parse_tree(txt):
    root = None; stack = []; bids = {}
    for ln in txt.split('\n'):
        if not ln.strip():
            continue
        m = LINE.match(ln)
        if not m:
            continue
        d = len(m.group(1)); role = m.group(3); name = (m.group(4) or '').replace("\\'", "'")
        pk = ' '.join(KEEP.findall(m.group(5) or ''))
        n = Node(role, name, pk, m.group(2))
        if m.group(2):
            bids[m.group(2)] = (role, name)
        while len(stack) > d:
            stack.pop()
        if stack:
            stack[-1].ch.append(n)
        elif root is None:
            root = n
        else:
            root.ch.append(n)
        stack.append(n)
    return root, bids

def finish(n):
    """post-order: structural hash h, content hash c, size"""
    for c in n.ch:
        finish(c)
    n.h = hash((n.role, n.name, tuple(c.h for c in n.ch)))
    n.c = hash((n.role, n.name, n.pk, tuple(c.c for c in n.ch)))
    own = len(n.name) + len(n.role) + 4 if (n.name or n.role not in SKIP_ROLES) else 0
    n.size = own + sum(c.size for c in n.ch)

def render(n, depth=0, out=None, parent_name=None):
    out = [] if out is None else out
    show = False
    if n.role == 'StaticText':
        if n.name.strip() and n.name != parent_name:
            out.append(' ' * depth + '"' + n.name[:200] + '"')
        return out
    if n.name or n.role not in SKIP_ROLES:
        out.append(' ' * depth + label(n.role, n.name)); show = True
    for c in n.ch:
        render(c, depth + (1 if show else 0), out, n.name)
    return out

def value_lines(n, out=None):
    out = [] if out is None else out
    if n.pk:
        out.append(label(n.role, n.name) + ' ' + n.pk)
    for c in n.ch:
        value_lines(c, out)
    return out

def flat_lines(n, out):
    if n.role == 'StaticText':
        if n.name.strip(): out.add('"' + n.name[:160] + '"')
    else:
        if n.name: out.add(label(n.role, n.name))
    for c in n.ch:
        flat_lines(c, out)

def url_template(u):
    u = unquote(u or '')
    u = re.sub(r'^https?://[^/]+', '', u)
    u = re.sub(r'=[^&?]*', '=', u)
    u = re.sub(r'[0-9a-f]{6,}', '<h>', u)
    u = re.sub(r'\d+', '#', u)
    return u[:140]

def title_template(t):
    t = re.sub(r'[0-9a-f]{8,}', '<h>', t or ''); t = re.sub(r'\d+', '#', t)
    return t[:90]

def parse_action(a):
    m = re.match(r"(\w+)\((.*)\)\s*$", a or '', re.S)
    if not m: return (a or '', ())
    try:
        v = ast.literal_eval('(' + m.group(2) + ',)')
    except Exception:
        v = (m.group(2),)
    return m.group(1), v

ERRW = re.compile(r"\b(error|invalid|not allowed|denied|failed|failure|cannot|unable|forbidden|exception|required field|not found|no results)\b", re.I)

def action_label(a, bids):
    f, args = parse_action(a)
    if f in ('click', 'hover', 'fill', 'select_option', 'dblclick', 'press', 'focus', 'check', 'uncheck', 'clear', 'upload_file', 'drag_and_drop') and args:
        r = bids.get(str(args[0]))
        tgt = label(*r) if r else f"bid {args[0]}"
        extra = ''
        if f in ('fill', 'select_option') and len(args) > 1:
            extra = f" <- '{str(args[1])[:60]}'"
        return f"{f} {tgt}{extra}"
    if f == 'scroll': return 'scroll'
    if f in ('keyboard_press',) and args: return f"press key {args[0]}"
    if f == 'goto' and args: return f"goto {url_template(str(args[0]))}"
    return f"{f}{str(args)[:60]}" if f else 'noop'

class Model:
    def __init__(s):
        s.units = {}          # h -> dict(text, bc, traj:set, nstates:int, vals:Counter, first)
        s.eff = {}            # (tmpl, label) -> dict
        s.eps = {}            # traj id -> text
        s.tmpl = collections.defaultdict(lambda: dict(n=0, traj=set(), titles=collections.Counter(), out=collections.Counter(), inv=collections.Counter()))
        s.raw_bytes = 0; s.unit_bytes_struct = 0; s.unit_occ = 0; s.unit_bytes_occ = 0; s.nstates = 0
        s.aut = collections.Counter()

    def add_traj(s, t):
        prev_lines = None; prev_bids = None; prev_tmpl = None; steps = []
        for st in t['states']:
            tree = st['accessibility_tree'] or ''
            s.raw_bytes += len(tree); s.nstates += 1
            root, bids = parse_tree(tree)
            if root is None:
                continue
            finish(root)
            title = root.name
            tm = (url_template(st['url']), title_template(title))
            td = s.tmpl[tm]; td['n'] += 1; td['traj'].add(t['id']); td['titles'][title[:80]] += 1
            lines = set(); flat_lines(root, lines)
            td['inv'].update(lines)
            s._units(root, [label(root.role, title)], t['id'], st['state_index'], tm)
            if prev_lines is not None and st['action'] is not None:
                lab = action_label(st['action'], prev_bids)
                add = sorted(lines - prev_lines); rem = sorted(prev_lines - lines)
                key = (prev_tmpl, lab)
                e = s.eff.setdefault(key, dict(n=0, add=collections.Counter(), rem=collections.Counter(), nxt=collections.Counter(), traj=set(), loop=0, err=collections.Counter()))
                e['n'] += 1; e['traj'].add(t['id']); e['nxt'][tm] += 1
                for l in add[:60]: e['add'][l] += 1
                for l in rem[:60]: e['rem'][l] += 1
                for l in add:
                    if ERRW.search(l): e['err'][l[:120]] += 1
                nochange = not add and not rem
                if nochange: e['loop'] += 1
                td_prev = s.tmpl[prev_tmpl]; td_prev['out'][lab] += 1
                steps.append((st['state_index'], lab, tm, nochange, [l for l in add if ERRW.search(l)][:2]))
            prev_lines, prev_bids, prev_tmpl = lines, bids, tm
        head = f"Trajectory {t['id']} [{t['outcome']}] goal: {t['goal'][:300]}"
        body = []
        for i, lab, tm, nc, er in steps:
            x = f"{i}. {lab}" + (' (no visible change)' if nc else '') + (f" (ERROR text: {'; '.join(er)})" if er else '')
            body.append(x)
        last = prev_tmpl[1] if prev_tmpl else ''
        s.eps[t['id']] = head + '\n' + '\n'.join(body) + f"\nended on: {last}"

    def _units(s, root, path, tid, sidx, tm):
        def flush(batch, path):
            if not batch: return
            h = hash(tuple(n.h for n in batch)); c = hash(tuple(n.c for n in batch))
            u = s.units.get(h)
            sz = sum(n.size for n in batch)
            s.unit_occ += 1; s.unit_bytes_occ += sz
            if u is None:
                lines = []
                for n in batch: render(n, 0, lines)
                text = '\n'.join(lines)
                if len(text.strip()) < 3: s.units[h] = None; return
                u = dict(text=text, bc=' > '.join(path[-3:])[:200], tm=tm, traj=set(), n=0, vals=collections.Counter(), cs=set())
                s.units[h] = u; s.unit_bytes_struct += len(text)
            if u is None: return
            u['n'] += 1; u['traj'].add(tid)
            if c not in u['cs'] and len(u['cs']) < 12:
                u['cs'].add(c)
                for n in batch:
                    for v in value_lines(n): u['vals'][v[:160]] += 1
        def group(nodes, path):
            batch = []; bs = 0
            for n in nodes:
                if n.size > UNIT_MAX and n.ch:
                    flush(batch, path); batch = []; bs = 0
                    group(n.ch, path + [label(n.role, n.name)])
                else:
                    if bs + n.size > UNIT_MAX:
                        flush(batch, path); batch = []; bs = 0
                    batch.append(n); bs += n.size
            flush(batch, path)
        group(root.ch, path)

# ---------------- retrieval ----------------
GENERIC_STOP = set('a an the of to in on at by for from with and or but is are was were be been it its this that these those as if then than so such into over after before when while what which who whom how do does did i me my our we you your he she they them their there here not no any some each one can could would should will may might also about between within during'.split())
BENCH_STOP = set("a an the of to in on is are was were be been it its this that these those and or for with as at by from what which who how do does did i my our we you your can could would should there their them then than into over after before when while if not no any some each one two three first second third last next many much more most also than about between within during page pages click clicks button buttons using use used action actions need want like would please answer final mark boxed short phrase phrases separated semicolon wrapped english e g".split())
GENERIC = True
def stem(w):
    for suf in ('ing', 'edly', 'ed', 'es', 's'):
        if len(w) > len(suf) + 3 and w.endswith(suf): return w[:-len(suf)]
    return w
def toks(txt):
    st = GENERIC_STOP if GENERIC else BENCH_STOP
    return [stem(w) for w in re.findall(r"[a-z0-9]+", txt.lower()) if w not in st and len(w) > 1]

BOILER = [r"Mark your final answer.*", r"Your final answer should.*", r"Action Space:.*", r"I am (?:working|using) (?:with|in|on)? ?our [^.]*?(?:portal|website|dashboard|forum|site)[^.]*\.", r"Given the following action space.*"]
def clean_q(q):
    if GENERIC: return q
    q = q.split('Action Space:')[0]
    for b in BOILER[:2]: q = re.sub(b, '', q, flags=re.S)
    return q

class BM25:
    def __init__(s, docs, k1=1.2, b=0.75):
        s.N = len(docs); s.inv = collections.defaultdict(list); s.dl = []
        for i, d in enumerate(docs):
            c = collections.Counter(d); s.dl.append(len(d))
            for w, f in c.items(): s.inv[w].append((i, f))
        s.avg = sum(s.dl) / max(1, s.N); s.k1, s.b = k1, b
        s.idf = {w: math.log(1 + (s.N - len(p) + .5) / (len(p) + .5)) for w, p in s.inv.items()}
    def search(s, qt, n=50):
        sc = collections.defaultdict(float)
        for w in sorted(set(qt)):
            for i, f in s.inv.get(w, ()):
                sc[i] += s.idf[w] * f * (s.k1 + 1) / (f + s.k1 * (1 - s.b + s.b * s.dl[i] / s.avg))
        return sorted(sc.items(), key=lambda x: (-x[1], x[0]))[:n]

class Index:
    def __init__(s, m):
        s.m = m
        s.uk = [h for h, u in m.units.items() if u]
        s.ud = [m.units[h] for h in s.uk]
        s.u_bm = BM25([toks(u['text'] + ' ' + u['bc'] + ' ' + u['tm'][1]) for u in s.ud])
        s.ek = list(m.eff)
        s.ed = [s.eff_text(k) for k in s.ek]
        s.e_bm = BM25([toks(t) for t in s.ed])
        s.tk = [k for k, v in m.tmpl.items() if v['n'] >= 1]
        s.t_bm = BM25([toks(k[1] + ' ' + k[0] + ' ' + ' '.join(m.tmpl[k]['inv'])) for k in s.tk])
        s.pk = list(m.eps)
        s.p_bm = BM25([toks(m.eps[k]) for k in s.pk])
    def eff_text(s, k):
        e = s.m.eff[k]; tm, lab = k
        add = [l for l, _ in e['add'].most_common(14)]; rem = [l for l, _ in e['rem'].most_common(6)]
        nxt = e['nxt'].most_common(1)[0][0][1] if e['nxt'] else ''
        x = f"On page '{tm[1]}': {lab} (x{e['n']}) -> "
        if e['loop'] == e['n']: x += 'no visible change (self-loop)'
        else:
            x += 'now shown: ' + '; '.join(add)
            if rem: x += ' | gone: ' + '; '.join(rem)
            x += f" | next page: '{nxt}'"
        if e['err']: x += ' | ERROR text: ' + '; '.join(list(e['err'])[:2])
        if 0 < e['loop'] < e['n']: x += f" | nondeterministic: {e['loop']} of {e['n']} no change"
        return x
    def unit_text(s, u):
        x = f"[page '{u['tm'][1]}' {u['tm'][0][:60]}; in {len(u['traj'])} trajs/{u['n']} states] {u['bc']}\n{u['text']}"
        if u['vals']:
            x += '\nvalues seen: ' + ' | '.join(v for v, _ in sorted(u['vals'].items(), key=lambda kv: (-kv[1], kv[0]))[:6])
        return x
    def tmpl_text(s, k, maxc=4500):
        v = s.m.tmpl[k]; inv = v['inv']; tot = sum(inv.values()); n1 = sum(1 for c in inv.values() if c == 1)
        head = f"PAGE '{k[1]}' [{k[0][:80]}] observed in {v['n']} states / {len(v['traj'])} trajectories; {len(inv)} distinct lines, {n1} seen once (unseen-mass estimate {n1 / max(1, tot):.3f}). Element inventory (line: states containing it):"
        items = [f"{l} x{c}" for l, c in sorted(inv.items(), key=lambda kv: (-kv[1], kv[0])) if len(l) <= 90]
        out = head; 
        for it in items:
            if len(out) + len(it) + 3 > maxc: break
            out += ' | ' + it
        return out
    def pack(s, q, budget_tokens, frac=(0.5, 0.25, 0.25, 0.0), cpt=3.5, parts='UEP', ret_units=False):
        qt = toks(clean_q(q)); B = int(budget_tokens * cpt); out = []; used = []
        spec = [('U', s.u_bm, lambda i: s.unit_text(s.ud[i])), ('E', s.e_bm, lambda i: s.ed[i]), ('P', s.p_bm, lambda i: s.m.eps[s.pk[i]])]
        spec.append(('T', s.t_bm, lambda i: s.tmpl_text(s.tk[i])))
        fr = {k: f for k, f in zip('UEPT', tuple(frac) + (0.0,) * 4)}
        tot = sum(fr[k] for k in parts); 
        for k, bm, rend in spec:
            if k not in parts: continue
            cap = int(B * fr[k] / tot); sz = 0; items = []
            for i, sc in bm.search(qt, 200):
                t = rend(i)
                if sz + len(t) > cap:
                    t = t[:max(0, cap - sz)] if sz < cap * 0.9 else ''
                    if not t: continue
                items.append(t); sz += len(t) + 1
                if sz >= cap: break
            out.append(f"## {k}\n" + '\n'.join(items))
        return '\n'.join(out)
