#!/usr/bin/env python3
"""Paired audit of the recorded evidence-oracle run ("60 of 104 against 60"), judge-free.

Scores existing reader outputs with the upstream phrase matcher on development questions only
(questions are filtered against held_ids.json on load, and every cache is cut to those ids before
scoring). No model is called.

    uv run python benchmarks/lme2/headroom/evidence_audit.py --root /Volumes/A/.hms-lb/lme2
"""
import argparse, json, sys
from pathlib import Path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="/Volumes/A/.hms-lb/lme2")
    ap.add_argument("--held", default=str(Path(__file__).resolve().parent.parent / "held_ids.json"))
    ap.add_argument("--out", default=str(Path(__file__).resolve().parent / "evidence_audit.json"))
    a = ap.parse_args()
    root = Path(a.root)
    held = set(json.loads(Path(a.held).read_text()))
    sys.path.insert(0, str(root / "upstream"))
    from evaluation.qa_eval_metrics import norm_phrase_set_match, norm_phrase_set_match_ordered

    qs = {}
    for line in Path(root / "data" / "questions.jsonl").read_text().splitlines():
        q = json.loads(line)
        if q["id"] not in held:
            qs[q["id"]] = q
    assert len(qs) == 228
    phrase = {k for k, q in qs.items() if q["eval_function"].startswith("norm_phrase")}

    def score(path):
        cache = json.loads(Path(root / path).read_text())
        out = {}
        for k, v in cache.items():
            if k not in phrase:
                continue
            q = qs[k]
            ans = q["answer"] if isinstance(q["answer"], str) else ";".join(q["answer"])
            f = norm_phrase_set_match_ordered if "ordered" in q["eval_function"] else norm_phrase_set_match
            out[k] = bool(f(v["output"].get("response_parsed_boxed"), ans))
        return out

    oracle = score("final-arch/rc_oracle2.json")
    base = score("final-arch/rc_arch_uep16.json")
    r6 = {}
    for line in Path(root / "final-arch" / "r6_per_question_dev.snapshot.jsonl").read_text().splitlines():
        d = json.loads(line)
        if d["question_id"] in phrase:
            r6[d["question_id"]] = float(d["score"]) >= 1.0

    def paired(other, name):
        ids = sorted(set(oracle) & set(other))
        c = {"retained": 0, "recovered": 0, "regressed": 0, "unresolved": 0}
        for k in ids:
            o, b = oracle[k], other[k]
            c["retained" if o and b else "recovered" if o else "regressed" if b else "unresolved"] += 1
        return {"control": name, "n": len(ids), "oracle_correct": sum(oracle[k] for k in ids),
                "control_correct": sum(other[k] for k in ids), **c}

    result = {
        "phrase_dev_questions": len(phrase),
        "oracle_feasible": len(oracle),
        "pairs": [paired(r6, "round 6 (wns_e: notes and support analysis by the reader model)"),
                  paired(base, "arch_uep16 pack without the oracle block (the pack oracle2 extends)")],
        "construction": "arch/oracle2.py: for each phrase-scored dev question, BM25 units then transitions of "
                        "the compiled index whose text contains a gold phrase, up to 9000 chars, until every gold "
                        "phrase is covered; questions where some phrase is in no unit or transition are dropped. "
                        "The block '## ORACLE EVIDENCE' is prepended to the arch_uep16 prompt row. Reader, prompt "
                        "template and sampling are those of the arch_uep16 rows; the r6 context (notes, support "
                        "analysis, 6 states) is not part of it.",
        "scoring": "upstream norm_phrase_set_match(_ordered) on response_parsed_boxed; no judge",
    }
    Path(a.out).write_text(json.dumps(result, indent=1) + "\n")
    print(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
