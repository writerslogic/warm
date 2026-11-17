# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tool layer for the SPAN arm: arch2 compiled-memory tools (index/manifest) + raw-state locators + submit_spans.
find_states: BM25 over per-state text (url+action+thought-free AXTree); show_state: preview of one state; submit_spans ends the loop."""
import tools2 as T2
import envmodel as EM
from envmodel import toks
from prompts import MAX_SPAN_STATES, EXTRA_SPECS, SUBMIT_SPANS

class Tools3(T2.Tools):
    def __init__(s, traj_ids, T, notes=None):
        super().__init__(traj_ids, T, notes)
        s.T = {i: T[i] for i in traj_ids}
        s.sk = []; docs = []
        for tid in traj_ids:
            for st in T[tid]['states']:
                s.sk.append((tid, int(st['state_index'])))
                docs.append(toks((st['url'] or '') + ' ' + str(st['action'] or '') + ' ' + (st['accessibility_tree'] or '')[:20000]))
        s.s_bm = EM.BM25(docs)
    def find_states(s, query, k=8):
        k = max(1, min(int(k or 8), 12)); out = []
        for i, sc in s.s_bm.search(toks(query), k):
            tid, si = s.sk[i]; st = s.T[tid]['states'][si]
            out.append(f"{tid} state {si} (of {len(s.T[tid]['states'])}) url={str(st['url'])[:80]} action={str(st['action'])[:80]} score={sc:.1f}")
        return '\n'.join(out) or 'no results'
    def show_state(s, trajectory_id, state_index, maxc=2500):
        try: st = s.T[trajectory_id]['states'][int(state_index)]
        except Exception: return "unknown trajectory_id or state_index (ids as in find_states / episode listings)"
        return f"{trajectory_id} state {st['state_index']} url={st['url']} action={st['action']}\n" + (st['accessibility_tree'] or '')[:int(maxc)]
    def goal(s, trajectory_id):
        t = s.T.get(trajectory_id); return f"{trajectory_id} [{t['outcome']}] {len(t['states'])} states; goal: {t['goal'][:300]}" if t else 'unknown trajectory'
    def dispatch(s, name, a):
        try:
            if name == 'find_states': return s.find_states(a.get('query', ''), a.get('k', 8))[:6500]
            if name == 'show_state': return s.show_state(a.get('trajectory_id', ''), a.get('state_index', 0), a.get('max_chars', 2500))[:6500]
            if name == 'trajectory_goal': return s.goal(a.get('trajectory_id', ''))
        except Exception as e: return f"tool error: {e!r}"[:200]
        return super().dispatch(name, a)
    def check_spans(s, spans):
        """returns (valid spans, total states, error text or None). Trims nothing: over budget is an error the model must fix."""
        v = []; tot = 0
        for sp in spans or []:
            try:
                tid = str(sp['trajectory_id']); a = int(sp['start_state_index']); b = int(sp['end_state_index'])
                n = len(s.T[tid]['states'])
                if not (0 <= a <= b < n): return None, 0, f"span {tid} {a}-{b} outside 0..{n-1}"
            except Exception as e: return None, 0, f"bad span {sp}: {e!r}"
            v.append({'trajectory_id': tid, 'start_state_index': a, 'end_state_index': b}); tot += b - a + 1
        if tot > MAX_SPAN_STATES: return None, tot, f"spans cover {tot} states, limit is {MAX_SPAN_STATES}; shrink or drop spans (least important last)"
        return v, tot, None

BASE = [t for t in T2.TOOL_SPECS if t["function"]["name"] != "submit_brief"]
SPAN_SPECS = BASE + EXTRA_SPECS + [SUBMIT_SPANS]
