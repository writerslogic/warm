from __future__ import annotations

import hashlib
import json
import math
import time
from collections import defaultdict
from pathlib import Path

import numpy as np

from memory_modules.memory import register_memory
from warm_hybrid import _tokens
from warm_slice import WarmSliceMemory

SUPPORT_SYSTEM = (
    "You are a memory retrieval module. You read evidence retrieved from recorded agent sessions in a customized web "
    "environment and prepare a short analysis for a downstream reader that answers a question about that environment."
)
SUPPORT_USER = """Question:
{question}

Retrieved evidence (whole page states from recorded sessions):
{evidence}

Write a Support Analysis of at most 250 words:
1. Evidence: the trajectory ids and state numbers that bear on the question, with the exact UI strings (labels, options, field names, values, order) you rely on, quoted literally.
2. Premise check: list every UI element, option, field, step, count, or ordering the question presupposes. For each, say whether the evidence shows it exists exactly as described. Recorded pages are complete: a control, option, or step that is absent from the relevant page, or whose position, count, or name differs from the question, means the premise is wrong.
3. Verdict, on its own last line, exactly one of:
   VERDICT: SUPPORTED <the answer the evidence gives, in the form the question asks>
   VERDICT: PREMISE_FALSE <what the question assumes and what the evidence shows instead>
   VERDICT: NOT_FOUND
Use only the evidence above. Do not guess."""


def qkey(query: str) -> str:
    return hashlib.sha256(query.encode()).hexdigest()[:20]


def _load(path):
    p = Path(path) if path else None
    return json.loads(p.read_text()) if p and p.exists() else {}


@register_memory
class WarmNotesMemory(WarmSliceMemory):
    """warm_slice plus per-trajectory LLM notes (ingest) and a query-time Support Analysis,
    both produced by the local reader model and read from caches; context is budgeted."""

    memory_type = "warm_notes"
    DEFAULTS = {
        **WarmSliceMemory.DEFAULTS,
        "notes_path": None, "support_cache": None, "support_pending": None,
        "notes_k": 3, "note_chars": 2200, "use_notes": True, "use_support": True,
        "expand_hits": 4, "support_states": 6, "support_chars": 90000, "state_order": "rank",
    }

    def __init__(self, memory_params):
        super().__init__(memory_params)
        self._notes = _load(self.p["notes_path"])
        self._support = _load(self.p["support_cache"])
        self._note_ids: list[tuple[str, str]] = []
        self._note_vecs = None
        self._note_post = None

    # ---- notes index
    def _note_text(self, tid, kind):
        n = self._notes[tid][kind]
        return f"Title: {n['title']}\nDescription: {n['description']}\n{n['content']}".strip()

    def _build_notes_index(self):
        if self._note_post is not None:
            return
        self._note_ids = [(t, k) for t in self._states if t in self._notes for k in ("procedure", "hint")]
        texts = [self._note_text(t, k) for t, k in self._note_ids]
        post = defaultdict(list)
        self._note_dl = np.asarray([max(1, len(_tokens(x))) for x in texts], np.float32)
        for i, x in enumerate(texts):
            tf = defaultdict(int)
            for tok in _tokens(x):
                tf[tok] += 1
            for tok, c in tf.items():
                post[tok].append((i, c))
        self._note_post = post
        self._note_avg = float(self._note_dl.mean()) if len(texts) else 1.0
        self._note_vecs = None
        if texts and self.p["dense"]:
            v = np.asarray(self._get_embedder().embed_passages(texts), np.float32)
            self._note_vecs = v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)

    def _top_notes(self, query, kind, k):
        self._build_notes_index()
        idx = [i for i, (_, kk) in enumerate(self._note_ids) if kk == kind]
        if not idx:
            return []
        n = len(self._note_ids)
        sc = np.zeros(n, np.float32)
        for tok in set(_tokens(query)):
            post = self._note_post.get(tok)
            if not post:
                continue
            idf = math.log(1 + (n - len(post) + 0.5) / (len(post) + 0.5))
            for d, c in post:
                sc[d] += idf * c * 2.2 / (c + 1.2 * (0.25 + 0.75 * self._note_dl[d] / self._note_avg))
        fused = defaultdict(float)
        for r, i in enumerate(sorted(idx, key=lambda i: -sc[i])):
            fused[i] += 1.0 / (60 + r + 1)
        if self._note_vecs is not None:
            q = np.asarray(self._get_embedder().embed_query(query), np.float32)
            q /= max(float(np.linalg.norm(q)), 1e-12)
            sims = self._note_vecs @ q
            for r, i in enumerate(sorted(idx, key=lambda i: -sims[i])):
                fused[i] += 1.0 / (60 + r + 1)
        return [self._note_ids[i][0] for i in sorted(fused, key=fused.get, reverse=True)[:k]]

    # ---- rendering
    def _plan(self, query):
        ranked = self._rank(query)[: int(self.p["top_states"])]
        by_traj: dict[str, set[int]] = defaultdict(set)
        order: list[str] = []
        for r, sid in enumerate(ranked):
            tid, pos = self._s_keys[sid]
            rad = int(self.p["radius"]) if r < int(self.p["expand_hits"]) else 0
            if tid not in by_traj:
                order.append(tid)
            n = len(self._states[tid])
            by_traj[tid].update(range(max(0, pos - rad), min(n, pos + rad + 1)))
        return ranked, order, by_traj

    def _render(self, order, by_traj, budget, cap):
        texts, used = [], 0
        for rank, tid in enumerate(order, 1):
            meta, sts = self._meta[tid], self._states[tid]
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
                texts.append((tid, text))
                used += len(text)
            if used >= budget:
                break
        return texts

    def query(self, query, query_image=None):
        t0 = time.perf_counter()
        self._finalize(); self._sfinal()
        p = self.p
        ranked, order, by_traj = self._plan(query)
        items = [{"type": "text", "value": "Retrieved memory from past agent trajectories:\n\n"}]
        if p["use_support"]:
            sranked = ranked[: int(p["support_states"])]
            so, sb = [], defaultdict(set)
            for r, sid in enumerate(sranked):
                tid, pos = self._s_keys[sid]
                if tid not in sb:
                    so.append(tid)
                n = len(self._states[tid]); rad = int(p["radius"]) if r < int(p["expand_hits"]) else 0
                sb[tid].update(range(max(0, pos - rad), min(n, pos + rad + 1)))
            ev = "\n".join(t for _, t in self._render(so, sb, int(p["support_chars"]), int(p["max_state_chars"])))
            key = qkey(query + "\x00" + ev)
            if self._support.get(key, "").strip():
                items.append({"type": "text", "value": "## Support Analysis\n" + self._support[key].strip() + "\n\n"})
            elif p["support_pending"] and key not in self._support:
                pend = Path(p["support_pending"])
                d = json.loads(pend.read_text()) if pend.exists() else {}
                d[key] = {"question": query, "evidence": ev}
                pend.write_text(json.dumps(d))
        if p["use_notes"] and self._notes:
            tids = list(order[: int(p["notes_k"])])
            for kind in ("procedure", "hint"):
                for t in self._top_notes(query, kind, int(p["notes_k"])):
                    if t not in tids:
                        tids.append(t)
            lines = []
            for t in tids:
                if t in self._notes:
                    for kind in ("procedure", "hint"):
                        lines.append(f"[{kind} note, trajectory {t}]\n{self._note_text(t, kind)[: int(p['note_chars'])]}")
            if lines:
                items.append({"type": "text", "value": "## Relevant Procedure and Hint Notes\n" + "\n\n".join(lines) + "\n\n"})
        centers: dict[str, list[int]] = defaultdict(list)
        for sid in ranked[: int(p["image_hits"])]:
            tid, pos = self._s_keys[sid]
            centers[tid].append(pos)
        for tid, text in self._render(order, by_traj, int(p["max_context_chars"]), int(p["max_state_chars"])):
            items.append({"type": "text", "value": text + "\n\n"})
            if p["screens_root"]:
                for pos in centers.get(tid, []):
                    shot = self._states[tid][pos].get("shot")
                    f = Path(str(p["screens_root"])) / tid / Path(str(shot)).name if shot else None
                    if f is not None and f.exists():
                        items.append({"type": "image", "value": str(f)})
        self.last_timing = {"total_s": time.perf_counter() - t0}
        return items
