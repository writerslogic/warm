# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Evidence-lifecycle fixes to G1, kept outside the frozen G1 files (check_frozen.py still pins those).
  Tools4          full-page index (20,000-char windows), show_state with an offset, find_states with page length,
                  value_timeline over the ordered observations of one control
  normalize_spans / merge_pass2_v2   unique (trajectory, state) accounting, adjacent ranges coalesced
  final_rule_v2   the frozen verdicts plus the basis of an `absent` verdict; a controller-only absent is revised when VERIFY
                  and CRUX both find the evidence
  apply_pass2_v2  pass-2 spans replace the evidence; an `absent` that rested only on the controller becomes `partial`
No labels are read anywhere in this module."""
from tools3 import Tools3
import envmodel as EM
from prompts import MAX_SPAN_STATES, EXTRA_SPECS, SUBMIT_SPANS, TOOL_SPECS
from rules import qkey

WINDOW = 20000


class Tools4(Tools3):
    def __init__(self, traj_ids, T, notes=None):
        super().__init__(traj_ids, T, notes)
        self.sk = []; docs = []
        for tid in traj_ids:
            for st in T[tid]['states']:
                head = (st['url'] or '') + ' ' + str(st['action'] or '') + ' '
                tree = st['accessibility_tree'] or ''
                for a in range(0, max(len(tree), 1), WINDOW):
                    self.sk.append((tid, int(st['state_index'])))
                    docs.append(EM.toks(head + tree[a:a + WINDOW]))
        self.s_bm = EM.BM25(docs)

    def find_states(self, query, k=8):
        k = max(1, min(int(k or 8), 12)); out = []; seen = set()
        for i, sc in self.s_bm.search(EM.toks(query), k * 4):
            tid, si = self.sk[i]
            if (tid, si) in seen: continue
            seen.add((tid, si)); st = self.T[tid]['states'][si]
            out.append(f"{tid} state {si} (of {len(self.T[tid]['states'])}) url={str(st['url'])[:80]} action={str(st['action'])[:80]} "
                       f"chars={len(st['accessibility_tree'] or '')} score={sc:.1f}")
            if len(seen) >= k: break
        return '\n'.join(out) or 'no results'

    def show_state(self, trajectory_id, state_index, maxc=2500, offset=0):
        try: st = self.T[trajectory_id]['states'][int(state_index)]
        except Exception: return "unknown trajectory_id or state_index (ids as in find_states / episode listings)"
        tree = st['accessibility_tree'] or ''; o = max(0, int(offset or 0)); m = max(1, int(maxc))
        more = f"\n[page text continues: {len(tree) - o - m} more chars; call show_state with offset={o + m}]" if o + m < len(tree) else ''
        return (f"{trajectory_id} state {st['state_index']} url={st['url']} action={st['action']} "
                f"chars {o}-{min(o + m, len(tree))} of {len(tree)}\n" + tree[o:o + m] + more)

    def value_timeline(self, trajectory_id, element, k=40):
        """Every logged observation of the controls of one trajectory whose label contains `element`, in state order.
        Not filtered by relevance: the first row is the first OBSERVATION, which is not necessarily the initial value."""
        el_q = str(element or '').strip().lower(); rows = []
        if not el_q: return "give an element label (substring)"
        for tid, si, el, v in self.vl:
            if tid == trajectory_id and el_q in el.lower(): rows.append((int(si), el, v))
        if not rows: return "no logged observations for that control in this trajectory"
        rows.sort(key=lambda r: (r[1], r[0])); out = []; cur = None; last = None
        for si, el, v in rows:
            if el != cur: out.append(f"{el[:70]} (first observed at step {si}):"); cur = el; last = None
            if v != last: out.append(f"  step {si}: {str(v)[:80]}"); last = v
        return '\n'.join(out[:max(1, min(int(k or 40), 80))])

    def dispatch(self, name, a):
        try:
            if name == 'find_states': return self.find_states(a.get('query', ''), a.get('k', 8))[:6500]
            if name == 'show_state': return self.show_state(a.get('trajectory_id', ''), a.get('state_index', 0), a.get('max_chars', 2500), a.get('offset', 0))[:6500]
            if name == 'value_timeline': return self.value_timeline(a.get('trajectory_id', ''), a.get('element', ''), a.get('k', 40))[:6500]
        except Exception as e: return f"tool error: {e!r}"[:200]
        return super().dispatch(name, a)


def _s(t): return {"type": t}


def _specs():
    out = []
    for sp in EXTRA_SPECS:
        sp = {"type": sp["type"], "function": dict(sp["function"])}
        if sp["function"]["name"] == "show_state":
            sp["function"]["description"] = ("Preview one raw state (url, action, a window of its page text, 2500 chars from offset 0 by default). "
                                             "Long pages continue: pass offset to read further windows. find_states reports each page's length.")
            props = dict(sp["function"]["parameters"]["properties"]); props["offset"] = _s("integer")
            sp["function"]["parameters"] = {**sp["function"]["parameters"], "properties": props}
        out.append(sp)
    out.append({"type": "function", "function": {
        "name": "value_timeline",
        "description": "Ordered observations of one control (by label substring) within one trajectory, every logged value in state order, not relevance-filtered. "
                       "The first row is the first observation, which is not necessarily the initial value.",
        "parameters": {"type": "object", "properties": {"trajectory_id": _s("string"), "element": _s("string"), "k": _s("integer")},
                       "required": ["trajectory_id", "element"]}}})
    return out


SPAN_SPECS_V2 = [t for t in TOOL_SPECS if t["function"]["name"] != "submit_brief"] + _specs() + [SUBMIT_SPANS]


def normalize_spans(spans):
    """Unique (trajectory, state) observations, trajectories in order of first appearance, adjacent states coalesced into ranges."""
    states = {}
    for sp in spans or []:
        tid = sp["trajectory_id"]; d = states.setdefault(tid, set())
        d.update(range(sp["start_state_index"], sp["end_state_index"] + 1))
    out = []
    for tid, d in states.items():
        run = None
        for si in sorted(d):
            if run is not None and si == run[1] + 1: run[1] = si
            else:
                if run: out.append({"trajectory_id": tid, "start_state_index": run[0], "end_state_index": run[1]})
                run = [si, si]
        if run: out.append({"trajectory_id": tid, "start_state_index": run[0], "end_state_index": run[1]})
    return out


def merge_pass2_v2(spans, first_pass, limit=MAX_SPAN_STATES):
    """Pass-2 spans first, then first-pass states not already covered, counted per unique state, at most `limit` states."""
    have = {(s["trajectory_id"], si) for s in normalize_spans(spans) for si in range(s["start_state_index"], s["end_state_index"] + 1)}
    keep = list(normalize_spans(spans)); budget = limit - len(have)
    for s in normalize_spans(first_pass):
        for si in range(s["start_state_index"], s["end_state_index"] + 1):
            if (s["trajectory_id"], si) in have or budget <= 0: continue
            have.add((s["trajectory_id"], si)); budget -= 1
            keep.append({"trajectory_id": s["trajectory_id"], "start_state_index": si, "end_state_index": si})
    return normalize_spans(keep)[:]


def final_rule_v2(C, V, X, Q):
    """The frozen final_rule verdicts, plus `absent_basis`. A verdict that rests only on the controller is revised to supported
    when VERIFY (premise not violated, support supported) and CRUX (not missing) both find the evidence."""
    out = {}
    for i, c in C.items():
        v = V.get(i) or {}; x = X.get(i) or {}
        basis = [n for n, hit in (("controller", c['support'] == 'absent'), ("verify", v.get('premise') == 'violated'),
                                  ("crux", x.get('verdict') == 'missing')) if hit]
        if basis == ["controller"] and v.get('support') == 'supported' and v.get('premise') != 'violated' and x.get('verdict') not in (None, 'missing'):
            an, sup, basis = v.get('analysis') or c['analysis'], 'supported', []
        elif c['support'] == 'absent' or v.get('premise') == 'violated': an, sup = v.get('analysis') or c['analysis'], 'absent'
        elif x.get('verdict') == 'missing': an, sup = x.get('note', ''), 'absent'
        else: an, sup = c['analysis'], c['support']
        out[qkey(Q[i])] = {'id': i, 'analysis': an, 'spans': c['spans'] or [], 'support': sup, 'absent_basis': basis, 'seconds': c['stats']['seconds']}
    return out


def apply_pass2_v2(base, P2, Q):
    """Pass-2 spans replace the evidence. The earlier verdict is stale when the new evidence differs: an `absent` that rested only on the
    controller becomes `partial` (so the answer check still runs); one backed by VERIFY or CRUX is kept with its own analysis."""
    S = {qkey(Q[i]): dict(v) for i, v in base.items()}
    for r in P2:
        if not r['spans']: continue
        s = S[qkey(Q[r['id']])]; s['spans'] = r['spans']
        if s['support'] == 'absent' and s.get('absent_basis', ['verify']) == ['controller']: s.update(support='partial', analysis=r['analysis'], absent_basis=[])
        elif s['support'] != 'absent': s['analysis'] = r['analysis']
    return S
