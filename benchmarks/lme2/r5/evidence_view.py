#!/usr/bin/env python3
"""Evaluator-only viewer for annotating complete source evidence of a development question.

    uv run python benchmarks/lme2/r5/evidence_view.py <qid>                 # question, gold, locator hits
    uv run python benchmarks/lme2/r5/evidence_view.py <qid> --grep "text"   # states of the haystack containing text
    uv run python benchmarks/lme2/r5/evidence_view.py --show <tid>/<state>  # one full state (AXTree, url, action, thought)

Refuses held-out ids. Output goes to the terminal only.
"""
import argparse, json, re, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path("/Volumes/A/.hms-lb/lme2")
ROWS = [ROOT / "round5/runs/slice12i_web/prompt_rows.jsonl", ROOT / "round5/runs/slice12i_enterprise/prompt_rows.jsonl"]


def trajectories(ids):
    out = {}
    with open(ROOT / "data/trajectories.jsonl") as f:
        for line in f:
            if line[7:15] in ids:
                t = json.loads(line)
                out[t["id"]] = t
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("qid", nargs="?")
    ap.add_argument("--grep", action="append", default=[])
    ap.add_argument("--show")
    ap.add_argument("--context", type=int, default=4)
    ap.add_argument("--max", type=int, default=12)
    a = ap.parse_args()
    held = set(json.loads(Path(HERE.parent / "held_ids.json").read_text()))
    if a.show:
        tid, st = a.show.split("/")
        t = trajectories({tid})[tid]
        s = next(s for s in t["states"] if int(s["state_index"]) == int(st))
        print(f"trajectory {tid} goal: {t['goal']}\noutcome: {t['outcome']}\nstate {st} step {s['step']} url {s['url']}\naction that led here: {s['action']}\nthought: {s['thought']}\nscreenshot: {s.get('screenshot')}\n--- AXTree\n{s['accessibility_tree']}")
        return
    if a.qid in held:
        sys.exit("held-out id refused")
    row = next(json.loads(l) for p in ROWS for l in Path(p).read_text().splitlines() if json.loads(l)["question_id"] == a.qid)
    loc = json.loads(Path(HERE / "evidence_locate.json").read_text())["per_question"].get(a.qid, {})
    print(f"id {a.qid}  type {row['question_type']}  eval {row['eval_function']}")
    print(f"question: {row['question_text']}")
    print(f"gold: {row['answer_gold']}")
    print(f"R5 correct: {loc.get('correct')}  stage: {loc.get('stage')}")
    text = "\n".join(i["value"] for i in row["memory_context"] if i["type"] == "text")
    sel = re.findall(r"- Trajectory: (\w+) |^State (\d+) \(step", text, re.M)
    cur, shown = None, []
    for tid, st in sel:
        if tid: cur = tid
        else: shown.append(f"{cur}/{st}")
    print("R5 rendered states:", " ".join(shown))
    print("R5 images:", " ".join("/".join(i["value"].split("/")[-2:]) for i in row["memory_context"] if i["type"] == "image"))
    trajs = trajectories(set(row["haystack_ids"]))
    needles = a.grep or [p for p in re.split(r"[,;]", row["answer_gold"]) if p.strip()]
    for nd in needles:
        nd = nd.strip()
        pat = re.compile(re.escape(nd), re.I)
        n = 0
        print(f"\n=== '{nd}'")
        for tid in row["haystack_ids"]:
            t = trajs[tid]
            for s in t["states"]:
                lines = (s["accessibility_tree"] or "").split("\n")
                hits = [k for k, l in enumerate(lines) if pat.search(l)]
                extra = [f for f in ("url", "action", "thought") if pat.search(str(s.get(f) or ""))]
                if not hits and not extra:
                    continue
                n += 1
                if n > a.max:
                    continue
                mark = " [R5 rendered]" if f"{tid}/{s['state_index']}" in shown else ""
                print(f"-- {tid}/{s['state_index']}{mark} ({t['outcome']}) goal: {t['goal'][:140]}")
                print(f"   url {s['url'][:140]}  led-by {s['action']}  also-in {extra}")
                for k in hits[:3]:
                    for l in lines[max(0, k - a.context): k + a.context + 1]:
                        print("   | " + l[:200])
                    print("   ...")
        print(f"=== '{nd}': {n} states")


if __name__ == "__main__":
    main()
