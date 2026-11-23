# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""R5 (`slice12i`) selection with a rendering that states the bindings the observations carry.

Selection, per-state cap, running character cut and the screenshot set are those of
`warm_slice.WarmSliceMemory` (`render: "r5"` reproduces its rows byte for byte). `render:
"bound"` changes only the presentation of the same kept text and the same images:

- each state is labelled with its trajectory, its position in it, and whether it was a retrieved
  hit or a neighbour of one;
- the action is labelled as the one that led to the state, with the element it acted on when its
  id occurs exactly once in the previous state's tree (otherwise no element is named);
- the thought is labelled as the agent's thought at this state, before its next action;
- each screenshot follows the text of its own state with a caption naming that state;
- text cut by the running budget is marked as cut.

Nothing is added that is not in the trajectory: no summary, no inference, no absence claim.
"""
from __future__ import annotations

import re
import time
from collections import defaultdict
from pathlib import Path

from memory_modules.memory import register_memory
from warm_slice import WarmSliceMemory

_BID_ARG = re.compile(r"^\s*\w+\(\s*['\"]([A-Za-z0-9_-]+)['\"]")


def _element(tree: str, bid: str) -> str | None:
    pat = re.compile(r"^\s*\[" + re.escape(bid) + r"\]\s+(\S+)(?: '((?:[^'\\]|\\.)*)')?")
    found = [m for m in (pat.match(line) for line in tree.split("\n")) if m]
    if len(found) != 1:
        return None
    role, name = found[0].group(1), found[0].group(2)
    return f"{role} '{name[:160]}'" if name else role


def action_target(states: list[dict], pos: int) -> str | None:
    """The element the action of state `pos` acted on, from the state before it."""
    act = states[pos].get("action")
    m = _BID_ARG.match(act or "")
    if not m or pos == 0:
        return None
    return _element(states[pos - 1]["tree"], m.group(1))


@register_memory
class WarmBoundMemory(WarmSliceMemory):
    memory_type = "warm_bound"

    DEFAULTS = {**WarmSliceMemory.DEFAULTS, "render": "bound"}

    @staticmethod
    def _r5_state(s, tree):
        return f"State {s['i']} (step {s['step']})\n- URL: {s['url']}\n- Action: {s['action'] or '<none>'}\n- Thought: {s['thought']}\n- AXTree:\n{tree}\n"

    def _bound_state(self, tid, sts, pos, tree, kept, hits):
        s = sts[pos]
        full = self._r5_state(s, tree)
        role = "retrieved hit" if pos in hits else "shown next to a retrieved hit"
        lines = [f"State {s['i']} (step {s['step']}) of trajectory {tid}, state {pos + 1} of {len(sts)}, {role}"]
        head = full[: full.index("- AXTree:\n") + len("- AXTree:\n")]
        if kept >= head.index("- Action:"):
            lines.append(f"- URL: {s['url']}")
        if kept >= head.index("- Thought:"):
            if s["action"]:
                tgt = action_target(sts, pos)
                on = f" (acted on state {sts[pos - 1]['i']}: {tgt})" if tgt else ""
                lines.append(f"- Action that led to this state: {s['action']}{on}")
            else:
                lines.append("- Action that led to this state: <none, first state>")
        if kept >= head.index("- AXTree:"):
            lines.append(f"- Agent's thought at this state, before its next action: {s['thought']}")
        body = "\n".join(lines) + "\n"
        if kept >= len(head):
            body += "- AXTree:\n" + full[len(head): kept]
        if kept < len(full):
            body = body.rstrip("\n") + "\n[... cut by the context budget]\n"
        return body

    def query(self, query, query_image=None):
        if self.p["render"] == "r5":
            return super().query(query, query_image)
        t0 = time.perf_counter()
        p = self.p
        ranked = self._rank(query)
        rad = int(p["radius"])
        by_traj: dict[str, set[int]] = defaultdict(set)
        hits: dict[str, set[int]] = defaultdict(set)
        order: list[str] = []
        for sid in ranked:
            tid, pos = self._s_keys[sid]
            if tid not in by_traj:
                order.append(tid)
            n = len(self._states[tid])
            by_traj[tid].update(range(max(0, pos - rad), min(n, pos + rad + 1)))
            hits[tid].add(pos)
        centers: dict[str, list[int]] = defaultdict(list)
        for sid in ranked[: int(p["image_hits"])]:
            tid, pos = self._s_keys[sid]
            centers[tid].append(pos)
        out = [{"type": "text", "value": "Retrieved memory from past agent trajectories:\n\n"}]
        used, budget, cap = 0, int(p["max_context_chars"]), int(p["max_state_chars"])
        any_text = False
        for rank, tid in enumerate(order, 1):
            meta, sts = self._meta[tid], self._states[tid]
            acts = "\n".join(f"{s['i']}: {s['action']}" for s in sts if s["action"])
            head = f"### Memory result {rank}\n- Trajectory: {tid} ({meta['outcome']})\n- Goal: {meta['goal_full']}\n\nFull action sequence\n{acts}\n"
            shown = sorted(by_traj[tid])
            trees = {pos: (sts[pos]["tree"] if len(sts[pos]["tree"]) <= cap else sts[pos]["tree"][:cap] + "\n[...truncated]") for pos in shown}
            pieces = [head] + [self._r5_state(sts[pos], trees[pos]) for pos in shown]
            # The R5 text of this result and where the running budget cuts it decide what is kept.
            limit = sum(map(len, pieces)) + len(pieces) - 1
            if used + limit > budget:
                limit = max(0, budget - used)
            if limit == 0 or not "\n".join(pieces)[:limit].strip():
                if used >= budget:
                    break
                continue
            any_text = True
            kept, off = [], 0
            for k, piece in enumerate(pieces):
                kept.append(max(0, min(len(piece), limit - off)))
                off += len(piece) + 1
            n_hits = sorted(h for h in hits[tid] if h in by_traj[tid])
            shown_line = (f"- States shown: {', '.join(str(sts[q]['i']) for q in shown)} of {len(sts)}"
                          f" (retrieved hits: {', '.join(str(sts[q]['i']) for q in n_hits)})\n")
            text = head[: kept[0]]
            if kept[0] == len(head):
                text = head.replace("\n\nFull action sequence\n", "\n" + shown_line + "\nFull action sequence (the action listed at state i led to state i)\n", 1)
            out.append({"type": "text", "value": text + "\n"})
            used += limit
            pics = [q for q in centers.get(tid, [])]
            placed = set()
            for k, pos in enumerate(shown, 1):
                if kept[k] == 0:
                    continue
                out.append({"type": "text", "value": self._bound_state(tid, sts, pos, trees[pos], kept[k], hits[tid]) + "\n"})
                if pos in pics:
                    self._shot(out, tid, sts, pos, "")
                    placed.add(pos)
            for pos in pics:
                if pos not in placed:
                    self._shot(out, tid, sts, pos, " (its page text is not shown: cut by the context budget)")
            out.append({"type": "text", "value": f"End of memory result {rank} (trajectory {tid}).\n\n"})
            if used >= budget:
                break
        self.last_timing = {"total_s": time.perf_counter() - t0}
        return out if any_text else []

    def _shot(self, out, tid, sts, pos, note):
        shot = sts[pos].get("shot")
        f = Path(str(self.p["screens_root"])) / tid / Path(str(shot)).name if shot and self.p["screens_root"] else None
        if f is not None and f.exists():
            s = sts[pos]
            out.append({"type": "text", "value": f"Screenshot of trajectory {tid}, state {s['i']} (step {s['step']}){note}:\n"})
            out.append({"type": "image", "value": str(f)})
