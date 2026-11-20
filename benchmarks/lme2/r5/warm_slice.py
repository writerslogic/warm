from __future__ import annotations

import math
import time
from collections import defaultdict

import numpy as np

from memory_modules.memory import MemoryContextItem, register_memory
from warm_hybrid import WarmHybridMemory, _tokens


@register_memory
class WarmSliceMemory(WarmHybridMemory):
    """State-level retrieval: chunk dense + chunk BM25 + full-state BM25 fused by RRF over
    states; the reader gets whole accessibility trees (center +- radius), not diffed fragments."""

    memory_type = "warm_slice"

    DEFAULTS = {
        **WarmHybridMemory.DEFAULTS,
        "top_states": 8,
        "radius": 1,
        "state_bm25_weight": 1.5,
        "max_context_chars": 240000,
        "max_state_chars": 60000,
        "pool_states": 60,
        "screens_root": None,
        "image_hits": 6,
    }

    def __init__(self, memory_params):
        super().__init__(memory_params)
        self._states: dict[str, list[dict]] = {}
        self._s_keys: list[tuple[str, int]] = []
        self._s_dl: list[int] = []
        self._s_pending: dict[str, list[tuple[int, int]]] = defaultdict(list)
        self._s_post = None

    def insert(self, trajectory):
        super().insert(trajectory)
        tid = str(trajectory["id"])
        goal = " ".join(str(trajectory.get("goal", "")).split())
        sts = []
        for st in trajectory.get("states", []):
            sts.append({
                "i": int(st.get("state_index", len(sts))), "step": st.get("step"), "url": st.get("url", ""),
                "action": st.get("action"), "thought": " ".join(str(st.get("thought") or "").split()),
                "tree": str(st.get("accessibility_tree") or ""), "shot": st.get("screenshot"),
            })
        self._states[tid] = sts
        self._meta[tid]["goal_full"] = goal
        for pos, s in enumerate(sts):
            toks = _tokens(f"{goal} {s['url']} {s['action']} {s['thought']} {s['tree']}")
            sid = len(self._s_keys)
            self._s_keys.append((tid, pos))
            self._s_dl.append(len(toks))
            tf: dict[str, int] = defaultdict(int)
            for t in toks:
                tf[t] += 1
            for t, c in tf.items():
                self._s_pending[t].append((sid, c))
        self._s_post = None

    def _sfinal(self):
        if self._s_post is None:
            self._s_post = {t: (np.fromiter((d for d, _ in v), np.int32, len(v)), np.fromiter((c for _, c in v), np.float32, len(v)))
                            for t, v in self._s_pending.items()}
            self._s_dlv = np.asarray(self._s_dl, np.float32)
            self._s_avg = float(self._s_dlv.mean()) if len(self._s_dlv) else 1.0

    def _state_bm25(self, query, pool):
        n = len(self._s_keys)
        sc = np.zeros(n, np.float32)
        for t in set(_tokens(query)):
            post = self._s_post.get(t)
            if post is None:
                continue
            docs, tf = post
            idf = math.log(1 + (n - len(docs) + 0.5) / (len(docs) + 0.5))
            sc[docs] += idf * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * self._s_dlv[docs] / self._s_avg))
        top = np.argpartition(-sc, min(pool, n - 1))[:pool]
        top = top[sc[top] > 0]
        return top[np.argsort(-sc[top])]

    def _rank(self, query):
        self._finalize()
        self._sfinal()
        p = self.p
        pool = int(p["pool_states"])
        k = int(p["rrf_k"])
        pos_of = {}
        for sid, (tid, pos) in enumerate(self._s_keys):
            pass
        index = {}
        for sid, key in enumerate(self._s_keys):
            index[key] = sid
        # chunk -> state score lists
        fused: dict[int, float] = defaultdict(float)

        def add_chunks(order, weight):
            seen = set()
            r = 0
            for d in order:
                step = self._chunk_step[int(d)]
                if step < 0:
                    continue
                sid = index.get((self._chunk_traj[int(d)], step))
                if sid is None or sid in seen:
                    continue
                seen.add(sid)
                fused[sid] += weight / (k + r + 1)
                r += 1

        if p["dense"] and self._emb is not None:
            q = np.asarray(self._get_embedder().embed_query(query), dtype=np.float32)
            q /= max(float(np.linalg.norm(q)), 1e-12)
            sims = self._emb @ q
            top = np.argpartition(-sims, min(pool * 3, len(sims) - 1))[: pool * 3]
            add_chunks(top[np.argsort(-sims[top])], float(p["dense_weight"]))
        if p["bm25"]:
            add_chunks(self._bm25(query, pool * 3), float(p["bm25_weight"]))
        for r, sid in enumerate(self._state_bm25(query, pool)):
            fused[int(sid)] += float(p["state_bm25_weight"]) / (k + r + 1)
        ranked = sorted(fused, key=fused.get, reverse=True)[: int(p["top_states"])]
        return ranked

    def query(self, query, query_image=None):
        t0 = time.perf_counter()
        p = self.p
        ranked = self._rank(query)
        rad = int(p["radius"])
        by_traj: dict[str, set[int]] = defaultdict(set)
        order: list[str] = []
        for sid in ranked:
            tid, pos = self._s_keys[sid]
            if tid not in by_traj:
                order.append(tid)
            n = len(self._states[tid])
            by_traj[tid].update(range(max(0, pos - rad), min(n, pos + rad + 1)))
        centers: dict[str, list[int]] = defaultdict(list)
        for sid in ranked[: int(p["image_hits"])]:
            tid, pos = self._s_keys[sid]
            centers[tid].append(pos)
        from pathlib import Path
        items: list = []
        parts, used, budget = [], 0, int(p["max_context_chars"])
        cap = int(p["max_state_chars"])
        for rank, tid in enumerate(order, 1):
            meta = self._meta[tid]
            sts = self._states[tid]
            acts = "\n".join(f"{s['i']}: {s['action']}" for s in sts if s["action"])
            blocks = [f"### Memory result {rank}\n- Trajectory: {tid} ({meta['outcome']})\n- Goal: {meta['goal_full']}\n\nFull action sequence\n{acts}\n"]
            for pos in sorted(by_traj[tid]):
                s = sts[pos]
                tree = s["tree"] if len(s["tree"]) <= cap else s["tree"][:cap] + "\n[...truncated]"
                blocks.append(f"State {s['i']} (step {s['step']})\n- URL: {s['url']}\n- Action: {s['action'] or '<none>'}\n- Thought: {s['thought']}\n- AXTree:\n{tree}\n")
            text = "\n".join(blocks)
            if used + len(text) > budget:
                text = text[: max(0, budget - used)]
            if text.strip():
                parts.append(text)
                items.append(("t", text))
                used += len(text)
                if p["screens_root"]:
                    for pos in centers.get(tid, []):
                        shot = self._states[tid][pos].get("shot")
                        f = Path(str(p["screens_root"])) / tid / Path(str(shot)).name if shot else None
                        if f is not None and f.exists():
                            items.append(("i", str(f)))
            if used >= budget:
                break
        self.last_timing = {"total_s": time.perf_counter() - t0}
        if not parts:
            return []
        out = [{"type": "text", "value": "Retrieved memory from past agent trajectories:\n\n"}]
        for kind, v in items:
            out.append({"type": "text", "value": v + "\n\n"} if kind == "t" else {"type": "image", "value": v})
        return out
