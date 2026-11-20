# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Invariants of evidence_v2: no labels, no model calls. usage: uv run python test_evidence_v2.py"""
import evidence_v2 as E
from tools3 import Tools3

def tr(tid, pages): return {tid: {'id': tid, 'goal': 'g', 'outcome': 'success', 'states': [
    {'state_index': str(i), 'step': str(i), 'url': 'u', 'action': 'a', 'thought': '', 'accessibility_tree': p, 'screenshot': None} for i, p in enumerate(pages)]}}

# a control past character 20,000 is findable and readable only through the windowed index and offset
filler = ' '.join(f'row{i} alpha' for i in range(4000)); assert len(filler) > 20000
T = tr('t1', [filler + ' zebraquux button', 'short page'])
for cls, expect in ((Tools3, False), (E.Tools4, True)):
    tl = cls(['t1'], T, None); hit = 'state 0' in tl.find_states('zebraquux', 3)
    assert hit == expect, (cls.__name__, hit)
t4 = E.Tools4(['t1'], T, None); n = len(filler) + len(' zebraquux button')
assert f'chars={n}' in t4.find_states('zebraquux', 3)
tail = t4.show_state('t1', 0, 2500, offset=n - 100); assert 'zebraquux' in tail and 'continues' not in tail
head = t4.show_state('t1', 0, 2500); assert 'continues' in head and 'offset=2500' in head

# overlapping spans are counted once and adjacent ranges coalesce
sp = lambda a, b, t='t1': {'trajectory_id': t, 'start_state_index': a, 'end_state_index': b}
assert E.normalize_spans([sp(0, 3), sp(2, 5), sp(6, 7), sp(9, 9)]) == [sp(0, 7), sp(9, 9)]
m = E.merge_pass2_v2([sp(0, 9)], [sp(5, 14), sp(30, 40)]); assert sum(s['end_state_index'] - s['start_state_index'] + 1 for s in m) == 20
m = E.merge_pass2_v2([sp(0, 1)], [sp(0, 1)]); assert m == [sp(0, 1)]

# verdict lifecycle
Q = {'q1': 'question one'}
C = {'q1': {'id': 'q1', 'support': 'absent', 'analysis': 'ctl', 'spans': [sp(0, 0)], 'stats': {'seconds': 1}}}
V = {'q1': {'premise': 'holds', 'support': 'supported', 'analysis': 'ver'}}; X = {'q1': {'verdict': 'found'}}
r = next(iter(E.final_rule_v2(C, V, X, Q).values())); assert r['support'] == 'supported' and r['absent_basis'] == []
r = next(iter(E.final_rule_v2(C, {'q1': {'premise': 'violated', 'analysis': 'v'}}, X, Q).values())); assert r['support'] == 'absent' and 'verify' in r['absent_basis']
r = next(iter(E.final_rule_v2(C, {}, {'q1': {'verdict': 'missing', 'note': 'n'}}, Q).values())); assert r['support'] == 'absent' and r['absent_basis'] == ['controller', 'crux']
base = {'q1': {'id': 'q1', 'support': 'absent', 'analysis': 'old', 'spans': [], 'absent_basis': ['controller']}}
out = next(iter(E.apply_pass2_v2(base, [{'id': 'q1', 'spans': [sp(1, 1)], 'analysis': 'new'}], Q).values())); assert out['support'] == 'partial' and out['analysis'] == 'new'
base['q1']['absent_basis'] = ['verify']
out = next(iter(E.apply_pass2_v2(base, [{'id': 'q1', 'spans': [sp(1, 1)], 'analysis': 'new'}], Q).values())); assert out['support'] == 'absent' and out['analysis'] == 'old'
print('ok')
