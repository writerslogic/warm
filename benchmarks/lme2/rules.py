#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Deterministic G1 stages (no model calls, no labels).
  final-rule     <ctl.jsonl...> --verify v.jsonl --crux x.jsonl --out spans_f1.json
                 c1 verdict: absent when the controller says absent or VERIFY premise is violated (VERIFY analysis)
                 or CRUX verdict is missing (CRUX note); otherwise controller analysis and support
  choice-ids     ids.json out.json: questions offering explicit choices (regex on the question text)
  undecided-ids  opt.jsonl out.json: option-checked questions with no option supported by a grounded quote
  answers        reader_cache.json out.jsonl: pass-1 reader answers for ANSWER-CHECK (boxed answer, else the last 1500 chars)
  mkp6           --base spans_f1.json [--pass2 pass2.jsonl --out-spans spans_s2.json] [--anscheck anscheck.jsonl --out-first reread_first.json]
                 pass-2 spans applied over f1 (support kept from f1; analysis kept when f1 said absent); Evidence Check text
                 for answers ANSWER-CHECK finds contradicted with >=1 grounded quote when the f1 verdict is not absent"""
import argparse, hashlib, json
import fileio  # noqa: E402
from prompts import CHOICE_PATTERN, evidence_check


def qkey(q): return hashlib.sha256(q.encode()).hexdigest()[:20]


def final_rule(C, V, X, Q):
    out = {}
    for i, c in C.items():
        v = V.get(i) or {}; x = X.get(i) or {}
        if c['support'] == 'absent' or v.get('premise') == 'violated': an, sup = v.get('analysis') or c['analysis'], 'absent'
        elif x.get('verdict') == 'missing': an, sup = x.get('note', ''), 'absent'
        else: an, sup = c['analysis'], c['support']
        out[qkey(Q[i])] = {'id': i, 'analysis': an, 'spans': c['spans'] or [], 'support': sup, 'seconds': c['stats']['seconds']}
    return out


def choice_ids(ids, Q): return [i for i in ids if CHOICE_PATTERN.search(Q[i]['question'])]


def undecided_ids(O): return [o['id'] for o in O if not any(g['verdict'] == 'supported' and g['quotes'] for g in o['grounded'])]


def merge_pass2(spans, first_pass):
    """Pass-2 spans first, then first-pass spans not already present (exact dict equality) while the total stays <= 20 states."""
    merged = list(spans or []); n = sum(s["end_state_index"] - s["start_state_index"] + 1 for s in merged)
    for s in first_pass or []:
        k = s["end_state_index"] - s["start_state_index"] + 1
        if s not in merged and n + k <= 20: merged.append(s); n += k
    return merged


def pass1_answers(cache):
    out = []
    for k, v in cache.items():
        b = (v['output']['response_parsed_boxed'] or '').strip()
        out.append({'id': k.split('|')[0], 'answer': b if b else (v['output']['response_raw'] or '')[-1500:]})
    return out


def reread_first(A, base, Q):
    first = {}
    for a in A:
        if (a['c'] or {}).get('verdict') == 'contradicted' and a['grounded'] and base[a['id']]['support'] != 'absent':
            first[qkey(Q[a['id']])] = evidence_check(a['grounded'])
    return first


def apply_pass2(base, P2, Q):
    S = {qkey(Q[i]): dict(v) for i, v in base.items()}
    for r in P2:
        if r['spans']: S[qkey(Q[r['id']])].update(spans=r['spans'], analysis=r['analysis'] if S[qkey(Q[r['id']])]['support'] != 'absent' else S[qkey(Q[r['id']])]['analysis'])
    return S


def _jsonl(p): return [json.loads(l) for l in fileio.lines(p)]


def main():
    import common
    ap = argparse.ArgumentParser(); sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("final-rule"); f.add_argument("ctl", nargs="+"); f.add_argument("--verify", required=True); f.add_argument("--crux", required=True); f.add_argument("--out", required=True)
    c = sub.add_parser("choice-ids"); c.add_argument("ids"); c.add_argument("out")
    u = sub.add_parser("undecided-ids"); u.add_argument("opt"); u.add_argument("out")
    w = sub.add_parser("answers"); w.add_argument("cache"); w.add_argument("out")
    m = sub.add_parser("mkp6"); m.add_argument("--base", required=True); m.add_argument("--pass2"); m.add_argument("--out-spans"); m.add_argument("--anscheck"); m.add_argument("--out-first")
    a = ap.parse_args()
    if a.cmd == "final-rule":
        Q = {i: q['question'] for i, q in common.questions().items()}
        C = common.load_jsonl_by_id(a.ctl)
        V = {r['id']: r['v'] for r in _jsonl(a.verify)}; X = {r['id']: r['c'] for r in _jsonl(a.crux)}
        out = final_rule(C, V, X, Q); fileio.write_json(out, a.out); print(a.out, len(out), 'absent', sum(o['support'] == 'absent' for o in out.values()))
    elif a.cmd == "choice-ids":
        out = choice_ids(fileio.read_json(a.ids), common.questions()); fileio.write_json(out, a.out); print(len(out))
    elif a.cmd == "undecided-ids":
        out = undecided_ids(_jsonl(a.opt)); fileio.write_json(out, a.out); print(len(out))
    elif a.cmd == "answers":
        with open(a.out, 'w') as f:
            for r in pass1_answers(fileio.read_json(a.cache)): f.write(json.dumps(r) + '\n')
    else:
        Q = {i: q['question'] for i, q in common.questions().items()}
        base = {v['id']: v for v in fileio.read_json(a.base).values()}
        if a.anscheck:
            A = _jsonl(a.anscheck); first = reread_first(A, base, Q)
            fileio.write_json(first, a.out_first); print('reread fires', len(first), 'of', len(A))
        if a.pass2:
            P2 = _jsonl(a.pass2); fileio.write_json(apply_pass2(base, P2, Q), a.out_spans); print('pass2 applied', len(P2))


if __name__ == "__main__":
    main()
