# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
# /// script
# requires-python = ">=3.13,<3.14"
# dependencies = ["numpy==2.2.6", "tiktoken==0.14.0"]
# ///
"""Reader-token cost of compact context renderings, and complete evidence under a fixed
reader-token budget, over the frozen dev native receipts. No model calls.

Budget per question: o200k tokens of the native selection in the official LongMemEval
flat-turn json layout (one chunk per round). Every arm keeps the whole native selection, so
complete evidence can only rise; it never spends more reader tokens than that budget.
"""

import argparse
import collections
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

import tiktoken

import longmemeval_retrieval as R
import longmemeval_retrieval_limits as L
import longmemeval_sota_receipts as N

ENC = tiktoken.get_encoding("o200k_base")
NICETIES = re.compile(
    r"(?im)(?:^|(?<=[.!?] ))(?:sure|certainly|absolutely|of course|great|wonderful|excellent|"
    r"that's (?:great|a great|wonderful|exciting|fantastic)|what a (?:great|wonderful)|"
    r"i'?m (?:glad|happy|sorry)|thank you|thanks|congratulations|i hope (?:this|that|these)|"
    r"hope (?:this|that) helps|let me know|feel free|i'?d be (?:happy|glad)|happy to help|"
    r"good luck|enjoy|have (?:a|fun))[^.!?\n]*[.!?]?[ \t]*")
MARKDOWN = re.compile(r"(?m)^[ \t]*(?:#{1,6}[ \t]*|[-*+•][ \t]+|\d+[.)][ \t]+|>[ \t]*)|\*\*|__|`")
ARTICLES = re.compile(r"(?i)\b(?:a|an|the)\s+")
FILLER = re.compile(r"(?i)\b(?:very|really|just|quite|truly|actually|basically|definitely|"
                    r"certainly|absolutely|extremely|incredibly|highly|totally|simply)\s+")
SPACE = re.compile(r"[ \t]*\n[ \t\n]*|[ \t]{2,}")
PHRASE = re.compile(r"\b[A-Z][a-z]+(?: [A-Z][a-z]+)+\b")


def tokens(text):
    return len(ENC.encode(text, disallowed_special=()))


def date(ms):
    return datetime.fromtimestamp(int(ms) / 1000, tz=timezone.utc).strftime("%Y/%m/%d (%a) %H:%M")


def official(sources, rounds):
    """run_generation's flat-turn json layout: one chunk per round, ordered by date."""
    chunks = collections.defaultdict(list)
    for s in sources:
        chunks[rounds[s["sourceId"]]].append(s)
    ordered = sorted(chunks.values(), key=lambda c: (int(c[0]["observationTime"]), c[0]["sessionId"], c[0]["turnIndex"]))
    out = ""
    for i, chunk in enumerate(ordered):
        chunk.sort(key=lambda s: s["turnIndex"])
        body = json.dumps([{"role": s["role"], "content": s["text"]} for s in chunk])
        out += f"\n### Session {i + 1}:\nSession Date: {date(chunk[0]['observationTime'])}\nSession Content:\n\n{body}\n"
    return out


def symbolic(sources):
    """One date header per session, one `U:`/`A:` line per turn, `...` where turns are elided."""
    sessions = collections.defaultdict(list)
    for s in sources:
        sessions[s["sessionId"]].append(s)
    ordered = sorted(sessions.values(), key=lambda c: (min(int(s["observationTime"]) for s in c), c[0]["sessionId"]))
    out = ""
    for chunk in ordered:
        chunk.sort(key=lambda s: s["turnIndex"])
        out += f"\n## {date(chunk[0]['observationTime'])}\n"
        previous = None
        for s in chunk:
            if previous is not None and int(s["turnIndex"]) > previous + 1:
                out += "...\n"
            out += ("U: " if s["role"] == "user" else "A: ") + s["text"].strip() + "\n"
            previous = int(s["turnIndex"])
    return out


def squeeze(text, articles=True, niceties=True):
    """Drop pleasantries, markdown, intensifiers and (optionally) articles; fold whitespace."""
    if niceties:
        while (shorter := NICETIES.sub("", text)) != text:
            text = shorter
    text = FILLER.sub("", MARKDOWN.sub("", text))
    if articles:
        text = ARTICLES.sub("", text)
    return SPACE.sub(lambda m: "; " if "\n" in m.group(0) else " ", text).strip(" ;")


def squeeze_user(text):
    """User turns keep pleasantries, which can carry content ("thanks to my sister...")."""
    return squeeze(text, niceties=False)


def legend(text):
    """Replace capitalised multi-word names seen three or more times with `$n`, defined once."""
    counts = collections.Counter(PHRASE.findall(text))
    names = [p for p, c in counts.most_common() if c >= 3 and len(p) >= 12]
    for n, phrase in enumerate(names, 1):
        text = text.replace(phrase, f"${n}")
    return ("defs:" + "|".join(f"${n}={p}" for n, p in enumerate(names, 1)) + "\n" + text) if names else text


def renderer(user, assistant, defs=False):
    def render(sources):
        out = symbolic([{**s, "text": (assistant if s["role"] == "assistant" else user)(s["text"])} for s in sources])
        return legend(out) if defs else out
    return render


def round_robin(ranked, available):
    """Interleave candidates across sessions, each session's in rank order."""
    queues = collections.OrderedDict()
    for sid in ranked:
        queues.setdefault(available[sid]["sessionId"], []).append(sid)
    out = []
    while queues:
        for key in list(queues):
            out.append(queues[key].pop(0))
            if not queues[key]:
                del queues[key]
    return out


def fill(order, available, budget, render, base, unit):
    """Admit each candidate's unit (itself, or its whole round) while the rendering fits."""
    chosen = {s["sourceId"]: s for s in base}
    for sid in order:
        if sid in chosen:
            continue
        trial = chosen | {s["sourceId"]: s for s in unit(sid)}
        if tokens(render(list(trial.values()))) <= budget:
            chosen = trial
    return list(chosen.values())


def study(entries, receipts):
    rows = []
    for qid, entry in sorted(entries.items()):
        groups, _, _, gt = R.rounds_for(entry)
        rounds = {s["sourceId"]: g["id"] for g in groups for s in g["sources"]}
        seed = receipts[qid]["replies"]["selected"]["evidence"]["sources"]
        pool = receipts[qid]["replies"]["candidate_ceiling"]["evidence"]
        available = {s["sourceId"]: s for s in pool["sources"]} | {s["sourceId"]: s for s in seed}
        by_round = collections.defaultdict(list)
        for s in available.values():
            by_round[rounds[s["sourceId"]]].append(s)
        ranked = [a["sourceId"] for a in sorted(pool["anchors"], key=lambda a: -a["score"])]
        aggregate = L.aggregation_intent(entry["question"])
        keep = (lambda t: t) if L.assistant_intent(entry["question"]) else squeeze
        renders = {
            "official": lambda sources: official(sources, rounds),
            "symbolic": symbolic,
            "squeeze_assistant": renderer(lambda t: t, keep),
            "squeeze_both_keep_articles": renderer(lambda t: squeeze(t, articles=False, niceties=False), keep),
            "squeeze_both": renderer(squeeze_user, keep),
            "squeeze_both_defs": renderer(squeeze_user, keep, defs=True),
        }
        budget = tokens(renders["official"](seed))
        arms = {name: (render, seed) for name, render in renders.items()}
        rounds_order = round_robin(ranked, available) if aggregate else ranked
        arms["round_fill"] = (renders["squeeze_both"], fill(
            rounds_order, available, budget, renders["squeeze_both"], seed, lambda sid: by_round[rounds[sid]]))
        users = [sid for sid in ranked if available[sid]["role"] == "user"]
        arms["user_fill"] = (renders["squeeze_both"], fill(
            round_robin(users, available) if aggregate else users, available, budget,
            renders["squeeze_both"], seed, lambda sid: [available[sid]]))
        row = {"qid": qid, "qtype": entry["question_type"], "aggregation_intent": aggregate, "budget_tokens": budget}
        for name, (render, chosen) in arms.items():
            ids = {s["sourceId"] for s in chosen}
            used = tokens(render(chosen))
            R.require(used <= budget or name in ("official", "symbolic"), f"{name} exceeds the reader-token budget")
            row[name] = {"tokens": used, "sources": len(chosen),
                         "complete": float(gt <= ids) if gt else None,
                         "coverage": len(gt & ids) / len(gt) if gt else None}
        rows.append(row)
    return {"summary": summarize(rows), "rows": rows}


def summarize(rows):
    arms = [k for k in rows[0] if isinstance(rows[0][k], dict)]
    scored = [r for r in rows if r["official"]["complete"] is not None]
    out = {"questions": len(rows), "scored": len(scored), "arms": {}}
    base = sum(r["official"]["tokens"] for r in rows)
    for arm in arms:
        by = collections.defaultdict(list)
        for r in scored:
            by[r["qtype"]].append(r[arm]["complete"])
        total = sum(r[arm]["tokens"] for r in rows)
        out["arms"][arm] = {
            "complete": sum(r[arm]["complete"] for r in scored) / len(scored),
            "tokens_mean": total / len(rows), "tokens_vs_official": total / base,
            "gained": sorted(r["qid"] for r in scored if r[arm]["complete"] > r["official"]["complete"]),
            "lost": sorted(r["qid"] for r in scored if r[arm]["complete"] < r["official"]["complete"]),
            "by_type": {k: f"{sum(v):.0f}/{len(v)}" for k, v in sorted(by.items())}}
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    entries = R.load_dev_conversations(R.SOURCE, R.SPLIT)
    result = study(entries, N.load(entries))
    result = {"schema_version": 1, "split": "dev", "dev_only": True, "llm_api_calls": 0,
              "receipts": str(N.DEFAULT.relative_to(R.ROOT)), "tokenizer": "o200k_base", **result}
    args.output.write_text(json.dumps(result, indent=1) + "\n")
    sys.stdout.write(json.dumps(result["summary"], indent=1) + "\n")


if __name__ == "__main__":
    main()
