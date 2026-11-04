#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Draw the statements to hand-label (protocol: docs/research/cue-tags.md).

usage: sample.py PROBE_JSONL LME_S_CLEANED_JSON SPLIT_JSON OUT_JSONL

STALE: DEV instances of the executed scope only (first 20 per type in probe order).
  stale_new: the M_new turn of every one of those 40 instances.
  stale_old: the M_old turn of 20 of them (seeded).
  stale_rand: 40 other user turns of those instances (seeded, conflict turns excluded).
LME-S: dev questions only. ku_new / ku_old: the latest-dated and earlier user turns carrying
  has_answer in the knowledge-update questions. lme_rand: other user turns of dev haystacks
  (seeded) to reach 100.
Text is cut to MAX_CHARS; labels and the tagger's P/R both use the cut text.
"""
import json, random, sys

MAX_CHARS = 800
SEED = 20261103

probe, lme_path, split_path, out = sys.argv[1:5]
rng = random.Random(SEED)
rows = []

insts = [json.loads(l) for l in open(probe)]
kept, dev = {}, []
for i in insts:
    if i["split"] != "dev":
        continue
    kept[i["type"]] = kept.get(i["type"], 0) + 1
    if kept[i["type"]] <= 20:
        dev.append(i)
assert len(dev) == 40
text = lambda i, tid: next(t["text"] for t in i["turns"] if t["id"] == tid)
for i in dev:
    rows.append(("stale_new", f'stale:{i["uid"]}:{i["new_id"]}', i["type"], text(i, i["new_id"])))
for i in rng.sample(dev, 20):
    rows.append(("stale_old", f'stale:{i["uid"]}:{i["old_id"]}', i["type"], text(i, i["old_id"])))
pool = [(i, t) for i in dev for t in i["turns"] if t["id"] not in (i["old_id"], i["new_id"])]
for i, t in rng.sample(pool, 40):
    rows.append(("stale_rand", f'stale:{i["uid"]}:{t["id"]}', i["type"], t["text"]))

dev_ids = set(json.load(open(split_path))["dev"])
lme = [e for e in json.load(open(lme_path)) if e["question_id"] in dev_ids]
assert len(lme) == 100
n_lme = 0
for e in lme:
    if e["question_type"] != "knowledge-update":
        continue
    ev = []
    for date, sid, sess in zip(e["haystack_dates"], e["haystack_session_ids"], e["haystack_sessions"]):
        for j, t in enumerate(sess):
            if t["role"] == "user" and t.get("has_answer"):
                ev.append((date, sid, j, t["content"]))
    ev.sort(key=lambda x: x[0])
    for k, (date, sid, j, c) in enumerate(ev):
        stratum = "ku_new" if k == len(ev) - 1 else "ku_old"
        rows.append((stratum, f'lme:{e["question_id"]}:{sid}:{j}', "knowledge-update", c))
        n_lme += 1
pool = [(e["question_id"], sid, j, t["content"]) for e in lme
        for sid, sess in zip(e["haystack_session_ids"], e["haystack_sessions"])
        for j, t in enumerate(sess) if t["role"] == "user" and not t.get("has_answer") and t["content"].strip()]
for qid, sid, j, c in rng.sample(pool, 100 - n_lme):
    rows.append(("lme_rand", f"lme:{qid}:{sid}:{j}", "haystack", c))

with open(out, "w") as f:
    for stratum, sid, kind, t in rows:
        f.write(json.dumps({"id": sid, "stratum": stratum, "kind": kind, "text": t[:MAX_CHARS]}) + "\n")
print(len(rows), {s: sum(r[0] == s for r in rows) for s in sorted({r[0] for r in rows})})
