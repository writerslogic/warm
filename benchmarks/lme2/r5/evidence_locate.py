#!/usr/bin/env python3
"""Evaluator-only: where R5 (`slice12i`) loses gold-phrase evidence on the locally scored dev questions.

For each phrase-scored question, every gold phrase (split and normalized by the question's own
eval spec, as the upstream matcher does) is searched in four places:

  observed   any state of any haystack trajectory (url, action, thought, accessibility tree)
  selected   the full source text of the states R5 rendered (parsed from the stored prompt rows)
  delivered  the text R5 actually sent, after its 60000-char state cap and 320000-char running cut
  answered   the reader's boxed answer (the upstream score)

A phrase found in text is a locator, not proof of sufficiency: it can match an unrelated element,
and a question can need relations the phrase does not carry. Multiple-choice questions have no
phrase to locate and get `unknown` for every stage. Screenshots are not searched; evidence that
is only visual is reported as not found in text, not as absent.

Output never contains question text, gold answers or reader text: only ids, stage flags, counts
and the state addresses where phrases occur.

    uv run python benchmarks/lme2/r5/evidence_locate.py --out benchmarks/lme2/r5/evidence_locate.json
"""
import argparse, json, re, sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
HEADER = re.compile(r"### Memory result \d+\n- Trajectory: (\w+) ")
STATE = re.compile(r"^State (\d+) \(step ", re.M)


def spec_kwargs(spec):
    import ast
    kw = {}
    for part in spec.split("|")[1:]:
        k, v = part.split("=", 1)
        kw[k.strip()] = v.strip()
    seps = kw.pop("separators", None)
    norm = {k: v.lower() == "true" for k, v in kw.items() if k in ("lower", "normalize_hyphen", "strip_punct")}
    return (list(seps) if seps else None), norm


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="/Volumes/A/.hms-lb/lme2")
    ap.add_argument("--held", default=str(HERE.parent / "held_ids.json"))
    ap.add_argument("--rows", nargs="+", default=[
        "/Volumes/A/.hms-lb/lme2/round5/runs/slice12i_web/prompt_rows.jsonl",
        "/Volumes/A/.hms-lb/lme2/round5/runs/slice12i_enterprise/prompt_rows.jsonl"])
    ap.add_argument("--scores", default=str(HERE / "r5_historical_local.json"))
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    root = Path(a.root)
    sys.path.insert(0, str(root / "upstream"))
    from evaluation.qa_eval_metrics import DEFAULT_SEPARATORS, normalize_phrase, split_phrases

    held = set(json.load(open(a.held)))
    scores = json.load(open(a.scores))["per_question"]
    rows = {}
    for p in a.rows:
        for line in open(p):
            r = json.loads(line)
            if r["question_id"] not in held and r["question_id"] in scores:
                rows[r["question_id"]] = r
    assert len(rows) == len(scores) == 149, (len(rows), len(scores))
    need = {t for r in rows.values() for t in r["haystack_ids"]}
    trajs = {}
    with open(root / "data" / "trajectories.jsonl") as f:
        for line in f:
            tid = line[7:15]
            if tid in need:
                t = json.loads(line)
                trajs[t["id"]] = t
    assert need <= set(trajs)

    norm_cache = {}

    def state_text(tid, pos, norm):
        key = (tid, pos, tuple(sorted(norm.items())))
        if key not in norm_cache:
            s = trajs[tid]["states"][pos]
            raw = f"{s.get('url','')} {s.get('action') or ''} {s.get('thought') or ''} {s.get('accessibility_tree') or ''}"
            norm_cache[key] = " " + normalize_phrase(raw, **norm) + " "
        return norm_cache[key]

    def has(text, ph):
        return re.search(r"\b%s\b" % re.escape(ph), text) is not None

    out = {}
    for qid, r in sorted(rows.items()):
        spec = r["eval_function"]
        sc = scores[qid]
        rec = {"question_type": r["question_type"], "eval": sc["eval"], "correct": sc["correct"]}
        if not spec.startswith("norm_phrase"):
            rec.update(observed="unknown", selected="unknown", delivered="unknown",
                       reason="multiple choice: no phrase to locate")
            out[qid] = rec
            continue
        seps, norm = spec_kwargs(spec)
        phrases = sorted(set(split_phrases(r["answer_gold"], separators=seps or DEFAULT_SEPARATORS, **norm)))
        texts = [i["value"] for i in r["memory_context"] if i["type"] == "text"]
        delivered = " " + normalize_phrase("\n".join(texts), **norm) + " "
        sel = []
        for block in "\n".join(texts).split("### Memory result ")[1:]:
            m = re.match(r"\d+\n- Trajectory: (\w+) ", block)
            for s in STATE.findall(block):
                pos = next((k for k, st in enumerate(trajs[m.group(1)]["states"])
                            if int(st.get("state_index", k)) == int(s)), None)
                if pos is not None:
                    sel.append((m.group(1), pos))
        truncated = sum("[...truncated]" in t for t in texts)
        per = []
        for ph in phrases:
            where = [(tid, pos) for tid in r["haystack_ids"] for pos in range(len(trajs[tid]["states"]))
                     if has(state_text(tid, pos, norm), ph)]
            per.append({
                "weak": len(ph) <= 3 or ph.isdigit() or len(where) > 40,
                "observed_states": len(where),
                "observed_trajectories": len({t for t, _ in where}),
                "selected": any(has(state_text(t, p, norm), ph) for t, p in sel),
                "delivered": has(delivered, ph),
                "first_locations": [f"{t}/{trajs[t]['states'][p].get('state_index', p)}" for t, p in where[:5]],
            })
        allf = lambda k: all(p[k] for p in per) if k != "observed_states" else all(p[k] > 0 for p in per)
        rec.update(
            phrases=len(phrases),
            weak_phrases=sum(p["weak"] for p in per),
            observed="yes" if allf("observed_states") else ("partial" if any(p["observed_states"] for p in per) else "not_in_text"),
            selected="yes" if allf("selected") else ("partial" if any(p["selected"] for p in per) else "no"),
            delivered="yes" if allf("delivered") else ("partial" if any(p["delivered"] for p in per) else "no"),
            selected_states=len(sel),
            states_over_cap=truncated,
            phrase_detail=per,
        )
        # A phrase in a selected state but not in what was sent was lost to the caps.
        rec["lost_to_clipping"] = any(p["selected"] and not p["delivered"] for p in per)
        out[qid] = rec

    def stage(rec):
        if rec["observed"] == "unknown":
            return "unknown (multiple choice)"
        if rec["observed"] != "yes":
            return "not fully observed in text"
        if rec["selected"] != "yes":
            return "observed, not selected"
        if rec["delivered"] != "yes":
            return "selected, lost to clipping"
        return "delivered (phrases present)"

    summary = defaultdict(lambda: {"n": 0, "correct": 0})
    for rec in out.values():
        k = stage(rec)
        rec["stage"] = k
        summary[k]["n"] += 1
        summary[k]["correct"] += rec["correct"]
    result = {"rows": a.rows, "n": len(out), "summary": dict(summary), "per_question": out}
    Path(a.out).write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")
    print(json.dumps(result["summary"], indent=1))


if __name__ == "__main__":
    main()
