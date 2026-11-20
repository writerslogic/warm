# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Query-driven state selection over R5's candidate pool, with the controls it is compared against.

Every variant uses R5's encoder, chunking, fused score (chunk dense + chunk BM25 + state BM25 by
RRF), 60-state pool, 12 hits, radius, caps and screenshot rule; only the choice of the 12 hits (or
the radius, for the expansion control) differs. No variant sees a gold answer, an annotation, a
question category or an evaluator.

select:
  r5        R5's top 12 by fused score (equals warm_slice).
  expand    control: R5's top 12 with radius 2, a deterministic temporal expansion.
  mmr       control: greedy maximal marginal relevance over the pool, penalising states whose page
            lines overlap an already chosen state (Jaccard), so near-duplicate pages are not
            chosen twice.
  pagecover candidate: greedy over the pool by fused score, at most one hit per page (URL path
            with ids and numbers masked, plus the page title with numbers masked) and at most
            `per_trajectory` hits per trajectory, so the 12 hits cover distinct pages. A page's
            identity text (title and URL path) gets its own BM25 term so that a question naming a
            page type ranks that page's states, not only states that mention its words.
"""
from __future__ import annotations

import math
import re
from collections import defaultdict
from urllib.parse import unquote, urlparse

import numpy as np

from memory_modules.memory import register_memory
from warm_bound import WarmBoundMemory
from warm_hybrid import _norm_line, _tokens

_NUM = re.compile(r"\d+")
_HEX = re.compile(r"\b[0-9a-f]{16,}\b")
_TITLE = re.compile(r"^RootWebArea '((?:[^'\\]|\\.)*)'")


def page_key(url: str, tree: str) -> tuple[str, str]:
    u = urlparse(url or "")
    path = _NUM.sub("#", _HEX.sub("*", unquote(u.path).lower()))
    m = _TITLE.match(tree or "")
    title = _NUM.sub("#", m.group(1).lower()) if m else ""
    return path, title


def identity_text(url: str, tree: str) -> str:
    u = urlparse(url or "")
    m = _TITLE.match(tree or "")
    return f"{unquote(u.path)} {m.group(1) if m else ''}".replace("_", " ").replace("/", " ").replace(".do", " ")


@register_memory
class WarmSelectMemory(WarmBoundMemory):
    memory_type = "warm_select"

    DEFAULTS = {**WarmBoundMemory.DEFAULTS, "render": "r5", "select": "r5", "mmr_lambda": 0.7,
                "per_trajectory": 3, "identity_weight": 1.0}

    def _fused(self, query):
        """R5's fused state scores, before its top-k cut (the body of warm_slice._rank)."""
        self._finalize()
        self._sfinal()
        p = self.p
        pool, k = int(p["pool_states"]), int(p["rrf_k"])
        index = {key: sid for sid, key in enumerate(self._s_keys)}
        fused: dict[int, float] = defaultdict(float)

        def add_chunks(order, weight):
            seen, r = set(), 0
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
        return fused, k

    def _identity_ranks(self, query, k):
        """RRF contribution of a BM25 over each state's page identity text (title and URL path)."""
        if not hasattr(self, "_id_post"):
            post: dict[str, list[int]] = defaultdict(list)
            lens = []
            for sid, (tid, pos) in enumerate(self._s_keys):
                s = self._states[tid][pos]
                toks = _tokens(identity_text(s["url"], s["tree"]))
                lens.append(len(toks))
                for t in set(toks):
                    post[t].append(sid)
            self._id_post = {t: np.asarray(v, np.int32) for t, v in post.items()}
            self._id_len = np.asarray(lens, np.float32)
        n = len(self._s_keys)
        sc = np.zeros(n, np.float32)
        avg = float(self._id_len.mean()) if n else 1.0
        for t in set(_tokens(query)):
            docs = self._id_post.get(t)
            if docs is None:
                continue
            idf = math.log(1 + (n - len(docs) + 0.5) / (len(docs) + 0.5))
            sc[docs] += idf * 2.2 / (1 + 1.2 * (0.25 + 0.75 * self._id_len[docs] / avg))
        pool = int(self.p["pool_states"])
        top = np.argpartition(-sc, min(pool, n - 1))[:pool]
        top = top[sc[top] > 0]
        return {int(s): float(self.p["identity_weight"]) / (k + r + 1) for r, s in enumerate(top[np.argsort(-sc[top])])}

    def _lines(self, sid):
        tid, pos = self._s_keys[sid]
        if not hasattr(self, "_line_cache"):
            self._line_cache = {}
        if sid not in self._line_cache:
            self._line_cache[sid] = frozenset(x for x in (_norm_line(l) for l in self._states[tid][pos]["tree"].splitlines()) if x)
        return self._line_cache[sid]

    def _rank(self, query):
        sel, top = self.p["select"], int(self.p["top_states"])
        if sel in ("r5", "expand"):
            return super()._rank(query)
        fused, k = self._fused(query)
        if sel == "mmr":
            pool = sorted(fused, key=fused.get, reverse=True)[: int(self.p["pool_states"])]
            hi = max(fused.values()) if fused else 1.0
            lam, chosen = float(self.p["mmr_lambda"]), []
            while pool and len(chosen) < top:
                def gain(s):
                    a = self._lines(s)
                    sim = max((len(a & self._lines(c)) / max(1, len(a | self._lines(c))) for c in chosen), default=0.0)
                    return lam * fused[s] / hi - (1 - lam) * sim
                best = max(pool, key=gain)
                chosen.append(best)
                pool.remove(best)
            return chosen
        if sel == "pagecover":
            for s, v in self._identity_ranks(query, k).items():
                fused[s] += v
            order = sorted(fused, key=fused.get, reverse=True)[: int(self.p["pool_states"]) * 2]
            pages, per, chosen = set(), defaultdict(int), []
            for s in order:
                tid, pos = self._s_keys[s]
                st = self._states[tid][pos]
                key = page_key(st["url"], st["tree"])
                if key in pages or per[tid] >= int(self.p["per_trajectory"]):
                    continue
                pages.add(key)
                per[tid] += 1
                chosen.append(s)
                if len(chosen) == top:
                    break
            return chosen
        raise ValueError(f"unknown select {sel}")
