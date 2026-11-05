#!/usr/bin/env python3
"""Slice the public STALE dataset (CC BY 4.0) into per-instance user-turn fact lists.

usage: prep.py STALE_JSON OUT_JSONL --dev-per-type N --test-per-type N [--seed S]

Eligible instance: M_old is an exact substring of a user turn in relevant_session_index[0] and
M_new is an exact substring of a user turn in relevant_session_index[1]. Facts are the user
turns of all 50 sessions, stamped with the session timestamp (UTC milliseconds).
Split is by instance, stratified by type, ordered by sha256(seed|uid).
"""
import argparse, calendar, hashlib, json, sys, time
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("src"); ap.add_argument("out")
ap.add_argument("--dev-per-type", type=int, required=True)
ap.add_argument("--test-per-type", type=int, required=True)
ap.add_argument("--seed", default="20261103")
a = ap.parse_args()

data = json.loads(Path(a.src).read_text())
elig = {"T1": [], "T2": []}
total = {"T1": 0, "T2": 0}
for inst in data:
    t = inst["type"]; total[t] += 1
    so, sn = inst["relevant_session_index"]
    def find(si, needle):
        for j, turn in enumerate(inst["haystack_session"][si]):
            if turn["role"] == "user" and needle in turn["content"]:
                return j
        return None
    jo, jn = find(so, inst["M_old"]), find(sn, inst["M_new"])
    if jo is None or jn is None:
        continue
    elig[t].append((hashlib.sha256((a.seed + "|" + inst["uid"]).encode()).hexdigest(), inst, so, jo, sn, jn))

def ts_ms(s):
    return calendar.timegm(time.strptime(s, "%Y-%m-%d %H:%M")) * 1000

summary = {"total": total, "eligible": {t: len(v) for t, v in elig.items()}, "dev": {}, "test": {}}
with open(a.out, "w") as f:
    for t, lst in elig.items():
        lst.sort(key=lambda x: x[0])
        picks = [("dev", x) for x in lst[: a.dev_per_type]] + [
            ("test", x) for x in lst[a.dev_per_type : a.dev_per_type + a.test_per_type]
        ]
        for split, (_, inst, so, jo, sn, jn) in picks:
            summary[split][t] = summary[split].get(t, 0) + 1
            turns = []
            for si, sess in enumerate(inst["haystack_session"]):
                ts = ts_ms(inst["timestamps"][si])
                for j, turn in enumerate(sess):
                    if turn["role"] == "user" and turn["content"].strip():
                        turns.append({"id": f"s{si:02d}t{j:02d}", "session": si, "ts": ts, "text": turn["content"]})
            pq = inst["probing_queries"]
            f.write(json.dumps({
                "uid": inst["uid"], "type": t, "split": split,
                "old_id": f"s{so:02d}t{jo:02d}", "new_id": f"s{sn:02d}t{jn:02d}",
                "old_session": so, "new_session": sn,
                "queries": {"dim1": pq["dim1_query"], "dim2": pq["dim2_query"], "dim3": pq["dim3_query"]},
                "turns": turns}) + "\n")
print(json.dumps(summary))
