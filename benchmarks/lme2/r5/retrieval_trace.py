#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Evaluator-only retrieval trace of every annotated evidence state of the pilot cohort, and the
admission control (how chunk hits become state candidates). Annotations score the trace; they are
never an input to a selector. No reader call.

Per required state (every annotated alternative set): its best chunk's rank over all chunks for
the dense and the chunk-BM25 channel and whether that is inside the 3 x pool chunk prefix each
channel keeps; its state-BM25 rank and admission to that channel's 60-state pool; its fused rank;
for R5 whether it is a top-12 centre or a radius-1 neighbour; for P whether it was chosen, skipped
by the one-per-page or per-trajectory limit, or never reached in P's scan.

Admission control (`--modes`): `r5` (frozen: top 180 chunks per channel, then collapse to states),
`wide` (top 600 chunks), `agg` (collapse every chunk to its state first, keep the top 180 states).
Encoder, query, 12 hits, radius, caps, images and R5 rendering are fixed. Reported per mode:
candidate states admitted, required states admitted to the fused pool, required states in the
final R5 and P selections, annotated evidence-state coverage of the rendered prompt, query time.

    uv run --python /Volumes/A/.hms-lb/lme2/venv/bin/python benchmarks/lme2/r5/retrieval_trace.py \
        --out benchmarks/lme2/r5/results/retrieval_trace_pilot.json
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from collections import defaultdict
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import prompts  # noqa: E402
import warm_select  # noqa: E402
from memory_modules.memory import MEMORY_TYPES  # noqa: E402
from selection_eval import delivered_states  # noqa: E402


def best_chunk_rank(mem, order):
    """state sid -> rank of its best chunk in a chunk order."""
    index = {key: sid for sid, key in enumerate(mem._s_keys)}
    out = {}
    for r, d in enumerate(order):
        step = mem._chunk_step[int(d)]
        if step < 0:
            continue
        sid = index.get((mem._chunk_traj[int(d)], step))
        if sid is not None and sid not in out:
            out[sid] = r
    return out


def pagecover_trace(mem, fused, k):
    """P's scan with the reason each scanned state was skipped (a copy of warm_select's loop)."""
    f = dict(fused)
    for s, v in mem._identity_ranks_for(k).items():
        f[s] = f.get(s, 0.0) + v
    order = sorted(f, key=f.get, reverse=True)[: int(mem.p["pool_states"]) * 2]
    pages, per, chosen, why = set(), defaultdict(int), [], {}
    for s in order:
        if len(chosen) == int(mem.p["top_states"]):
            break
        tid, pos = mem._s_keys[s]
        st = mem._states[tid][pos]
        key = warm_select.page_key(st["url"], st["tree"])
        if key in pages:
            why[s] = "page_limit"
            continue
        if per[tid] >= int(mem.p["per_trajectory"]):
            why[s] = "trajectory_limit"
            continue
        pages.add(key)
        per[tid] += 1
        chosen.append(s)
        why[s] = "chosen"
    return chosen, why


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--modes", nargs="*", default=["r5", "wide", "agg"])
    a = ap.parse_args()
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    cohort = [q for q in json.load(open(HERE / "pilot_cohort.json"))["ids"] if q not in held]
    ann = {}
    for f in sorted((HERE / "annotations").glob("batch_*.json")):
        for r in json.load(open(f)):
            if r["qid"] in cohort and r.get("sufficient_in_observations") == "yes":
                ann[r["qid"]] = r
    rows = {}
    for d in ("web", "enterprise"):
        for line in open(prompts.LME2 / f"round5/runs/slice12i_{d}/prompt_rows.jsonl"):
            r = json.loads(line)
            if r["question_id"] in ann:
                rows[r["question_id"]] = (d, r)
    need = {t for _, r in rows.values() for t in r["haystack_ids"]}
    trajs = {}
    with open(prompts.LME2 / "data/trajectories.jsonl") as f:
        for line in f:
            if line[7:15] in need:
                t = json.loads(line)
                trajs[t["id"]] = t
    trace, control = {}, {}
    for mode in a.modes:
        agg = {"admitted_states": [], "required": 0, "req_admitted": 0, "req_r5_selected": 0, "req_p_selected": 0,
               "coverage_r5": 0, "coverage_p": 0, "query_s": [], "per_question": {}}
        for d in ("web", "enterprise"):
            qs = [(q, r) for q, (dom, r) in rows.items() if dom == d]
            if not qs:
                continue
            base = {"cache_dir": str(prompts.LME2 / "embed_cache"), **prompts.R5, "admission": mode}
            mem = MEMORY_TYPES["warm_select"]({**base, "select": "r5"})
            memp = MEMORY_TYPES["warm_select"]({**base, "select": "pagecover"})
            for tid in qs[0][1]["haystack_ids"]:
                mem.insert(trajs[tid])
                memp.insert(trajs[tid])
            for q, r in qs:
                text = r["question_text"]
                t0 = time.perf_counter()
                fused, k = mem._fused(text)
                dt = time.perf_counter() - t0
                admitted = dict(mem.admitted)
                order = sorted(fused, key=fused.get, reverse=True)
                frank = {s: i for i, s in enumerate(order)}
                top = order[: int(mem.p["top_states"])]
                centers = {mem._s_keys[s] for s in top}
                memp._fused(text)
                memp._identity_ranks_for = lambda kk, m=memp, t=text: m._identity_ranks(t, kk)
                pchosen, why = pagecover_trace(memp, *memp._fused(text))
                pcenters = {memp._s_keys[s] for s in pchosen}
                key = {}
                for sid, (tid, pos) in enumerate(mem._s_keys):
                    key[(tid, int(mem._states[tid][pos]["i"]))] = sid
                if mode == "r5":
                    q_emb = np.asarray(mem._get_embedder().embed_query(text), dtype=np.float32)
                    sims = mem._emb @ (q_emb / max(float(np.linalg.norm(q_emb)), 1e-12))
                    dense_rank = best_chunk_rank(mem, np.argsort(-sims))
                    bm_rank = best_chunk_rank(mem, mem._bm25(text, len(mem._chunk_step)))
                    sb = {int(s): i for i, s in enumerate(mem._state_bm25(text, len(mem._s_keys)))}
                _, shown_r5 = delivered_states(mem.query(text, r["question_image"]))
                _, shown_p = delivered_states(memp.query(text, r["question_image"]))
                sets_out = []
                for es in ann[q]["evidence_sets"]:
                    states = []
                    for s in es["states"]:
                        tid, i = s.split("/")[0], int(s.split("/")[1])
                        sid = key.get((tid, i))
                        pos = mem._s_keys[sid][1] if sid is not None else None
                        near = sid is not None and any(t == tid and abs(p - pos) <= int(mem.p["radius"]) for t, p in centers)
                        nearp = sid is not None and any(t == tid and abs(p - pos) <= int(memp.p["radius"]) for t, p in pcenters)
                        rec = {"state": s, "admitted": {ch: sid in set(v) for ch, v in admitted.items()},
                               "fused_rank": frank.get(sid),
                               "r5": "centre" if (tid, pos) in centers else "neighbour" if near else "not_selected",
                               "p": "centre" if (tid, pos) in pcenters else ("neighbour" if nearp else why.get(sid, "not_reached")),
                               "rendered_r5": s in shown_r5, "rendered_p": s in shown_p}
                        if mode == "r5":
                            rec.update(dense_chunk_rank=dense_rank.get(sid), bm25_chunk_rank=bm_rank.get(sid),
                                       state_bm25_rank=sb.get(sid))
                        states.append(rec)
                        agg["required"] += 1
                        agg["req_admitted"] += sid in fused
                        agg["req_r5_selected"] += rec["r5"] != "not_selected"
                        agg["req_p_selected"] += rec["p"] in ("centre", "neighbour")
                    sets_out.append(states)
                cov_r5 = any(all(x["rendered_r5"] for x in st) for st in sets_out)
                cov_p = any(all(x["rendered_p"] for x in st) for st in sets_out)
                agg["coverage_r5"] += cov_r5
                agg["coverage_p"] += cov_p
                agg["admitted_states"].append(len(fused))
                agg["query_s"].append(dt)
                agg["per_question"][q] = {"admitted_states": len(fused), "coverage_r5": cov_r5, "coverage_p": cov_p}
                if mode == "r5":
                    trace[q] = sets_out
        n = len(agg["per_question"])
        control[mode] = {
            "questions": n, "mean_admitted_states": round(sum(agg["admitted_states"]) / n, 1),
            "required_states": agg["required"], "required_admitted_to_fused_pool": agg["req_admitted"],
            "required_in_r5_selection": agg["req_r5_selected"], "required_in_p_selection": agg["req_p_selected"],
            "annotated_evidence_state_coverage_r5": agg["coverage_r5"], "annotated_evidence_state_coverage_p": agg["coverage_p"],
            "mean_fusion_query_ms": round(1000 * sum(agg["query_s"]) / n, 2), "per_question": agg["per_question"],
        }
        sys.stdout.write(json.dumps({mode: {k2: v for k2, v in control[mode].items() if k2 != "per_question"}}) + "\n")
    Path(a.out).write_text(json.dumps({"trace": trace, "admission_control": control}, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
