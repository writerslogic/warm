#!/usr/bin/env python3
"""Score reader outputs on the locally scorable development questions, judge-free.

Uses the pinned upstream evaluator (`evaluation.qa_eval_metrics.eval_from_spec`) exactly as the
harness does for non-LLM eval functions: the boxed answer against the gold answer, and an
`is_unknown` answer scores 0. Questions whose eval function needs the hosted judge
(`llm_abstention_checker`, `llm_gotchas_checker`) are listed separately and never scored here.
Held-out ids are removed before any cache entry is read.

    uv run python benchmarks/lme2/r5/score_local.py --cache <reader_cache.json> --out <per_question.json>
"""
import argparse, json, sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
JUDGE = {"llm_abstention_checker", "llm_gotchas_checker"}


def load_dev(root: Path, held_path: Path):
    held = set(json.load(open(held_path)))
    qs = {}
    for line in open(root / "data" / "questions.jsonl"):
        q = json.loads(line)
        if q["id"] not in held:
            qs[q["id"]] = q
    assert len(qs) == 228, len(qs)
    return qs


def gold(q):
    return q["answer"] if isinstance(q["answer"], str) else ";".join(q["answer"])


def local_ids(qs):
    return sorted(k for k, q in qs.items() if q["eval_function"].split("|")[0] not in JUDGE)


def score_cache(cache: dict, qs: dict, eval_from_spec, score_to_bool):
    out = {}
    for k in local_ids(qs):
        v = cache.get(k)
        if v is None:
            continue
        o = v["output"]
        q = qs[k]
        ok = score_to_bool(eval_from_spec(q["eval_function"], o.get("response_parsed_boxed"), gold(q)))
        if o.get("is_unknown"):
            ok = False
        out[k] = {"correct": bool(ok), "eval": q["eval_function"].split("|")[0],
                  "question_type": q["question_type"], "unknown": bool(o.get("is_unknown"))}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="/Volumes/A/.hms-lb/lme2")
    ap.add_argument("--held", default=str(HERE.parent / "held_ids.json"))
    ap.add_argument("--cache", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    root = Path(a.root)
    sys.path.insert(0, str(root / "upstream"))
    from evaluation.qa_eval_metrics import eval_from_spec, score_to_bool

    qs = load_dev(root, Path(a.held))
    raw = json.load(open(a.cache))
    cache = {k: v for k, v in raw.items() if k in qs}
    res = score_cache(cache, qs, eval_from_spec, score_to_bool)
    judge = sorted(k for k in qs if k not in set(local_ids(qs)))
    by = Counter()
    for r in res.values():
        by[(r["question_type"], r["eval"], "n")] += 1
        by[(r["question_type"], r["eval"], "ok")] += r["correct"]
    summary = {
        "cache": str(a.cache),
        "local_questions": len(local_ids(qs)),
        "scored": len(res),
        "correct": sum(r["correct"] for r in res.values()),
        "judge_required_not_scored": len(judge),
        "by_type": {f"{t}/{e}": [by[(t, e, "ok")], by[(t, e, "n")]] for (t, e, x) in sorted(by) if x == "n"},
    }
    Path(a.out).write_text(json.dumps({"summary": summary, "per_question": res}, indent=1, sort_keys=True) + "\n")
    print(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
