from __future__ import annotations

import hashlib
import math
import re
import time
from collections import defaultdict
from pathlib import Path
from urllib.parse import unquote, urlparse

import numpy as np

from memory_modules.memory import Memory, MemoryContextItem, register_memory, require

_TOKEN = re.compile(r"[A-Za-z0-9_]+")
_SUFFIXES = ("ations", "ation", "ingly", "ings", "ing", "edly", "ed", "ies", "es", "ly", "s")
_NODE_ID = re.compile(r"^(\s*)\[\d+\]\s?")
_STOP = frozenset(
    "a an and are as at be by for from has have in is it its of on or that the this to was were will with".split()
)


def _stem(tok: str) -> str:
    if len(tok) <= 4 or any(c.isdigit() for c in tok):
        return tok
    for suf in _SUFFIXES:
        if tok.endswith(suf) and len(tok) - len(suf) >= 3:
            return tok[: -len(suf)]
    return tok


def _tokens(text: str) -> list[str]:
    return [_stem(t) for t in (m.lower() for m in _TOKEN.findall(text)) if t not in _STOP]


def _norm_line(line: str) -> str:
    return _NODE_ID.sub(r"\1", line).strip()


@register_memory
class WarmHybridMemory(Memory):
    """Chunked trajectory memory: WARM local embedder (exact cosine) fused with stemmed BM25."""

    memory_type = "warm_hybrid"

    DEFAULTS: dict[str, object] = {
        "cache_dir": None,
        "chunk_chars": 1400,
        "dense_pool": 80,
        "bm25_pool": 80,
        "rrf_k": 60,
        "dense_weight": 1.0,
        "bm25_weight": 1.0,
        "top_chunks": 14,
        "max_context_chars": 24000,
        "neighbors": 0,
        "dense": True,
        "bm25": True,
    }

    def __init__(self, memory_params: dict[str, object]) -> None:
        super().__init__(memory_params)
        self.p = {**self.DEFAULTS, **self.memory_params}
        self._embedder = None
        self._meta: dict[str, dict] = {}
        self._texts: list[str] = []
        self._chunk_traj: list[str] = []
        self._chunk_step: list[int] = []
        self._emb_parts: list[np.ndarray] = []
        self._emb: np.ndarray | None = None
        self._postings: dict[str, tuple[np.ndarray, np.ndarray]] | None = None
        self._pending_post: dict[str, list[tuple[int, int]]] = defaultdict(list)
        self._doc_len: list[int] = []
        self.last_timing: dict[str, float] = {}

    def _get_embedder(self):
        if self._embedder is None:
            import writerslogic_warm as w

            self._embedder = w.Embedder()
        return self._embedder

    @staticmethod
    def _page(url: str) -> str:
        u = urlparse(url or "")
        return unquote(u.path.rstrip("/").split("/")[-1])[:80] or u.netloc

    def _trajectory_chunks(self, traj: dict) -> list[tuple[str, int]]:
        limit = int(self.p["chunk_chars"])
        tid = traj["id"]
        goal = " ".join(str(traj.get("goal", "")).split())
        head = f"[{tid}] ({traj.get('outcome')})"
        out: list[tuple[str, int]] = []
        trace_lines = [f"{head} GOAL: {goal}"]
        prev: set[str] = set()
        for st in traj.get("states", []):
            i = int(st.get("state_index", 0))
            action = st.get("action")
            thought = " ".join(str(st.get("thought") or "").split())
            tree = str(st.get("accessibility_tree") or "")
            lines = [_norm_line(x) for x in tree.splitlines()]
            lines = [x for x in lines if x]
            novel = [x for x in lines if x not in prev]
            prev = set(lines)
            trace_lines.append(f"step {i} {self._page(st.get('url', ''))} | {action} | {thought}"[:420])
            title = lines[0][:120] if lines else ""
            base = f"{head} step {i} page: {title} action: {action} thought: {thought}"[:600]
            buf = base
            for ln in novel:
                if len(buf) + len(ln) + 1 > limit:
                    out.append((buf, i))
                    buf = base[:200] + " (cont.)"
                buf += "\n" + ln[:400]
            if buf:
                out.append((buf, i))
        buf = ""
        for ln in trace_lines:
            if len(buf) + len(ln) + 1 > limit:
                out.append((buf, -1))
                buf = f"{head} trace (cont.)"
            buf += ("\n" if buf else "") + ln
        if buf:
            out.append((buf, -1))
        seen: set[str] = set()
        uniq = []
        for text, step in out:
            h = hashlib.blake2b(text.encode(), digest_size=8).digest()
            if h not in seen:
                seen.add(h)
                uniq.append((text, step))
        return uniq

    def _embed_cached(self, tid: str, texts: list[str]) -> np.ndarray:
        cache_dir = self.p["cache_dir"]
        path = None
        if cache_dir:
            path = Path(str(cache_dir)) / f"{tid}.npy"
            sig = hashlib.blake2b("\x00".join(texts).encode(), digest_size=8).hexdigest()
            sigp = path.with_suffix(".sig")
            if path.exists() and sigp.exists() and sigp.read_text() == sig:
                return np.load(path)
        vecs = np.asarray(self._get_embedder().embed_passages(texts), dtype=np.float32)
        if path is not None:
            path.parent.mkdir(parents=True, exist_ok=True)
            np.save(path, vecs)
            sigp.write_text(sig)
        return vecs

    def insert(self, trajectory: dict[str, object]) -> None:
        tid = str(trajectory["id"])
        require(tid not in self._meta, f"duplicate trajectory {tid}")
        chunks = self._trajectory_chunks(trajectory)
        self._meta[tid] = {"goal": " ".join(str(trajectory.get("goal", "")).split())[:300], "outcome": trajectory.get("outcome")}
        base = len(self._texts)
        for k, (text, step) in enumerate(chunks):
            self._texts.append(text)
            self._chunk_traj.append(tid)
            self._chunk_step.append(step)
            toks = _tokens(text)
            self._doc_len.append(len(toks))
            tf: dict[str, int] = defaultdict(int)
            for t in toks:
                tf[t] += 1
            for t, c in tf.items():
                self._pending_post[t].append((base + k, c))
        if self.p["dense"]:
            self._emb_parts.append(self._embed_cached(tid, [c[0] for c in chunks]))
        self._emb = None
        self._postings = None

    def _finalize(self) -> None:
        if self._emb is None and self._emb_parts:
            m = np.concatenate(self._emb_parts, axis=0)
            norm = np.linalg.norm(m, axis=1, keepdims=True)
            self._emb = m / np.maximum(norm, 1e-12)
        if self._postings is None:
            self._postings = {
                t: (
                    np.fromiter((d for d, _ in v), dtype=np.int32, count=len(v)),
                    np.fromiter((c for _, c in v), dtype=np.float32, count=len(v)),
                )
                for t, v in self._pending_post.items()
            }
            self._dl = np.asarray(self._doc_len, dtype=np.float32)
            self._avgdl = float(self._dl.mean()) if len(self._dl) else 1.0

    def _bm25(self, query: str, pool: int) -> np.ndarray:
        n = len(self._texts)
        scores = np.zeros(n, dtype=np.float32)
        k1, b = 1.2, 0.75
        for t in set(_tokens(query)):
            post = self._postings.get(t)
            if post is None:
                continue
            docs, tf = post
            idf = math.log(1 + (n - len(docs) + 0.5) / (len(docs) + 0.5))
            denom = tf + k1 * (1 - b + b * self._dl[docs] / self._avgdl)
            scores[docs] += idf * tf * (k1 + 1) / denom
        top = np.argpartition(-scores, min(pool, n - 1))[:pool]
        top = top[scores[top] > 0]
        return top[np.argsort(-scores[top])]

    def query(self, query: str, query_image: str | None = None) -> list[MemoryContextItem]:
        t0 = time.perf_counter()
        self._finalize()
        p = self.p
        fused: dict[int, float] = defaultdict(float)
        if p["dense"] and self._emb is not None:
            q = np.asarray(self._get_embedder().embed_query(query), dtype=np.float32)
            q /= max(float(np.linalg.norm(q)), 1e-12)
            sims = self._emb @ q
            pool = int(p["dense_pool"])
            top = np.argpartition(-sims, min(pool, len(sims) - 1))[:pool]
            for r, d in enumerate(top[np.argsort(-sims[top])]):
                fused[int(d)] += float(p["dense_weight"]) / (int(p["rrf_k"]) + r + 1)
        t1 = time.perf_counter()
        if p["bm25"]:
            for r, d in enumerate(self._bm25(query, int(p["bm25_pool"]))):
                fused[int(d)] += float(p["bm25_weight"]) / (int(p["rrf_k"]) + r + 1)
        ranked = sorted(fused, key=fused.get, reverse=True)[: int(p["top_chunks"])]
        nb = int(p["neighbors"])
        chosen = list(ranked)
        if nb:
            have = set(chosen)
            for d in ranked:
                for off in range(1, nb + 1):
                    for e in (d - off, d + off):
                        if 0 <= e < len(self._texts) and self._chunk_traj[e] == self._chunk_traj[d] and e not in have:
                            have.add(e)
                            chosen.append(e)
        by_traj: dict[str, list[int]] = defaultdict(list)
        order: list[str] = []
        for d in chosen:
            tid = self._chunk_traj[d]
            if tid not in by_traj:
                order.append(tid)
            by_traj[tid].append(d)
        parts: list[str] = []
        used = 0
        budget = int(p["max_context_chars"])
        for tid in order:
            meta = self._meta[tid]
            block = [f"== Trajectory {tid} ({meta['outcome']}); goal: {meta['goal']}"]
            for d in sorted(by_traj[tid]):
                block.append(self._texts[d])
            text = "\n".join(block)
            if used + len(text) > budget:
                text = text[: max(0, budget - used)]
            if text.strip():
                parts.append(text)
                used += len(text)
            if used >= budget:
                break
        self.last_timing = {"dense_s": t1 - t0, "total_s": time.perf_counter() - t0}
        if not parts:
            return []
        return [{"type": "text", "value": "Retrieved memory from past agent trajectories:\n\n" + "\n\n".join(parts)}]
