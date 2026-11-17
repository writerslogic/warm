# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Shared inputs for the G1 stages: dataset paths from the environment, the small-tier trajectories, and the
state rendering and quote grounding used by VERIFY, CRUX, OPTCHECK and ANSWER-CHECK.

Environment:
  LME2_DATA        LongMemEval-V2 data dir (questions.jsonl, haystacks/, trajectories.jsonl, screenshots/)
  LME2_UPSTREAM    checkout of the upstream LongMemEval-V2 repository (evaluation/, memory_modules/)
  LME2_TRAJ_CACHE  pickle cache of the small-tier trajectories (default $LME2_DATA/lme2_small_trajectories.pkl)
  LME2_NOTES       per-trajectory procedure/hint notes (notes_stage.py output), used by the controller tools
"""
import json, os, pickle, re, sys
import fileio  # noqa: E402
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _env_path(name):
    v = os.environ.get(name)
    if not v:
        raise SystemExit(f"set {name}")
    return Path(v).expanduser().resolve()


def data_dir():
    return _env_path("LME2_DATA")


def upstream_dir():
    return _env_path("LME2_UPSTREAM")


def add_upstream_path():
    p = str(upstream_dir())
    if p not in sys.path:
        sys.path.insert(0, p)


def questions():
    return {json.loads(l)["id"]: json.loads(l) for l in fileio.lines(data_dir() / "questions.jsonl")}


def haystack():
    return fileio.read_json(data_dir() / "haystacks/lme_v2_small.json")


def notes():
    return fileio.read_json(_env_path("LME2_NOTES"))


_ID = re.compile(r'^\{"id":\s*"([^"]+)"')


def trajectories():
    """Small-tier trajectories {id: trajectory}, records unchanged from trajectories.jsonl."""
    cache = Path(os.environ.get("LME2_TRAJ_CACHE") or data_dir() / "lme2_small_trajectories.pkl")
    if cache.exists():
        return fileio.read_pickle(cache)
    want = {t for ids in haystack().values() for t in ids}
    out = {}
    with open(data_dir() / "trajectories.jsonl") as f:
        for line in f:
            m = _ID.match(line)
            if m and m.group(1) not in want:
                continue
            t = json.loads(line)
            if t["id"] in want:
                out[t["id"]] = t
    missing = want - set(out)
    if missing:
        raise SystemExit(f"{len(missing)} haystack trajectories missing from trajectories.jsonl")
    tmp = cache.with_suffix(".tmp")
    with open(tmp, "wb") as f:
        pickle.dump(out, f, protocol=4)
    tmp.replace(cache)
    return out


def load_jsonl_by_id(paths):
    out = {}
    for f in paths:
        for l in fileio.lines(f):
            r = json.loads(l); out[r["id"]] = r
    return out


def done_ids(out):
    return {json.loads(l)["id"] for l in fileio.lines(out)} if out.exists() else set()


# Rendering and grounding below are the reference stage functions unchanged; they read the module global T.
T = None
norm = lambda s: re.sub(r"\s+", " ", str(s or "")).strip().lower()
from prompts import PER, TOT  # noqa: E402


def init():
    global T
    if T is None:
        T = trajectories()
    return T


def state_text(st): return f"{st['url']} {st['action']} {st['accessibility_tree'] or ''}"
def render(r):
    parts = []; left = TOT
    for sp in r["spans"] or []:
        t = T[sp["trajectory_id"]]
        parts.append(f"=== trajectory {t['id']} [{t['outcome']}] goal: {str(t['goal'])[:300]}")
        for st in t["states"][sp["start_state_index"]: sp["end_state_index"] + 1]:
            tx = (st["accessibility_tree"] or "")[:min(PER, max(left, 0))]; left -= len(tx)
            parts.append(f"--- state {st['state_index']} url={st['url']} action={str(st['action'])[:200]}\n{tx}")
    return "\n".join(parts)
def ground(r, quotes):
    sel = {}
    for sp in r["spans"] or []:
        for st in T[sp["trajectory_id"]]["states"][sp["start_state_index"]: sp["end_state_index"] + 1]: sel[(sp["trajectory_id"], int(st["state_index"]))] = norm(state_text(st))
    ok = []
    for qt in quotes or []:
        k = (str(qt.get("trajectory_id")), int(qt.get("state_index", -1))); tx = re.sub(r"^(action|url)\s*[=:]\s*", "", norm(qt.get("text")))
        if len(tx) >= 2 and k in sel and tx in sel[k]: ok.append({"trajectory_id": k[0], "state_index": k[1], "text": re.sub(r"\s+", " ", qt["text"]).strip()})
    return ok
