#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Cue precision/recall against labels.csv, and selectivity over all user turns.

usage: measure.py SAMPLE_JSONL LABELS_CSV PROBE_JSONL LME_S_CLEANED_JSON SPLIT_JSON OUT_JSON
"""
import csv, json, sys
import io
from pathlib import Path
from collections import defaultdict

from cue_tagger import flagged, hits

sample_path, labels_path, probe, lme_path, split_path, out = sys.argv[1:7]
sample = {r["id"]: r for r in map(json.loads, Path(sample_path).read_text().splitlines())}
labels = list(csv.DictReader(io.StringIO(Path(labels_path).read_text())))
assert len(labels) == len(sample) == 200


def pr(items):
    tp = sum(1 for f, y in items if f and y)
    fp = sum(1 for f, y in items if f and not y)
    fn = sum(1 for f, y in items if not f and y)
    return {"n": len(items), "yes": tp + fn, "flagged": tp + fp, "tp": tp, "fp": fp, "fn": fn,
            "precision": tp / (tp + fp) if tp + fp else None,
            "recall": tp / (tp + fn) if tp + fn else None}


res = {"labels": {}, "selectivity": {}}
for mode in ("primary", "strict"):
    # strict: borderline ("weak") YES labels counted as NO.
    items, by = [], defaultdict(list)
    for r in labels:
        y = r["label"] == "yes" and not (mode == "strict" and r["note"] == "weak")
        f = flagged(sample[r["id"]]["text"])
        items.append((f, y))
        by[r["stratum"]].append((f, y))
    res["labels"][mode] = {"all": pr(items), **{s: pr(v) for s, v in sorted(by.items())},
                           "stale": pr(by["stale_new"] + by["stale_old"] + by["stale_rand"]),
                           "lme": pr(by["ku_new"] + by["ku_old"] + by["lme_rand"])}
rand = [r for r in labels if r["stratum"] in ("stale_rand", "lme_rand")]
res["base_rate"] = {
    "random_strata_yes": sum(r["label"] == "yes" for r in rand) / len(rand),
    "random_strata_yes_strict": sum(r["label"] == "yes" and r["note"] != "weak" for r in rand) / len(rand),
    "n": len(rand)}
newer = [r for r in labels if r["stratum"] in ("stale_new", "ku_new")]
res["fire_on_newer_conflict_statement"] = {
    s: sum(flagged(sample[r["id"]]["text"]) for r in newer if r["stratum"] == s) / sum(r["stratum"] == s for r in newer)
    for s in ("stale_new", "ku_new")}
by_type = defaultdict(list)
for r in labels:
    if r["stratum"] == "stale_new":
        by_type[sample[r["id"]]["kind"]].append(flagged(sample[r["id"]]["text"]))
res["fire_on_stale_new_by_type"] = {t: sum(v) / len(v) for t, v in sorted(by_type.items())}

# Selectivity: executed scope of the probe (first 20 dev / 40 test per type, file order).
kept, n, fired, group = defaultdict(int), 0, 0, defaultdict(int)
flags_by_uid = {}
for inst in map(json.loads, Path(probe).read_text().splitlines()):
    key = (inst["split"], inst["type"])
    kept[key] += 1
    if kept[key] > (20 if inst["split"] == "dev" else 40):
        continue
    ids = []
    for t in inst["turns"]:
        n += 1
        h = hits(t["text"])
        if h["lex"] or h["tense"]:
            fired += 1
            ids.append(t["id"])
        group["lex"] += bool(h["lex"])
        group["tense"] += bool(h["tense"])
    flags_by_uid[inst["uid"]] = ids
res["selectivity"]["stale_executed_scope"] = {"turns": n, "flagged": fired, "rate": fired / n,
                                              "lex_rate": group["lex"] / n, "tense_rate": group["tense"] / n,
                                              "instances": len(flags_by_uid)}
dev_ids = set(json.loads(Path(split_path).read_text())["dev"])
n = fired = 0
group = defaultdict(int)
for e in json.loads(Path(lme_path).read_text()):
    if e["question_id"] not in dev_ids:
        continue
    for sess in e["haystack_sessions"]:
        for t in sess:
            if t["role"] != "user" or not t["content"].strip():
                continue
            n += 1
            h = hits(t["content"])
            fired += bool(h["lex"] or h["tense"])
            group["lex"] += bool(h["lex"])
            group["tense"] += bool(h["tense"])
res["selectivity"]["lme_s_dev_haystacks"] = {"turns": n, "flagged": fired, "rate": fired / n,
                                             "lex_rate": group["lex"] / n, "tense_rate": group["tense"] / n}

p, r = res["labels"]["primary"]["all"]["precision"], res["labels"]["primary"]["all"]["recall"]
sel = max(v["rate"] for v in res["selectivity"].values())
res["kill"] = {"precision_lt_0.50": p < 0.50, "recall_lt_0.60": r < 0.60, "selectivity_gt_0.40": sel > 0.40}
res["survives"] = not any(res["kill"].values())
Path(out).write_text(json.dumps(res, indent=1))
Path(out.replace(".json", "_flags.json")).write_text(json.dumps(flags_by_uid))
print(json.dumps({k: res[k] for k in ("base_rate", "fire_on_newer_conflict_statement", "fire_on_stale_new_by_type", "selectivity", "kill", "survives")}, indent=1))
print(json.dumps({m: {k: res["labels"][m][k] for k in ("all", "stale", "lme")} for m in res["labels"]}, indent=1))
