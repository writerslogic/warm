# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""G1 memory module for the upstream harness: raw trajectory spans selected by the controller stages, rendered as upstream
codex/agentrunbook_c does (format_span_header + format_state_text, evidence_mode 'both' => AXTree text and screenshot),
with the Evidence Check first (reread rows), the support analysis and verdict, a form-control table before each state's page
text, and a 140000-token budget filled in controller order (it changes only contexts over budget).

memory_params: spans (final spans json keyed by question hash), first (reread Evidence Check json, pass 2 only),
form_backend (engine | reference), screens_root, store_root (engine store parent dir), mode (span_v)."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from memory_modules.memory import Memory, register_memory
from memory_modules.codex import format_span_header, format_state_text, MAX_TOTAL_SPAN_STATES
import formtable
from prompts import SUPPORT_VERDICT, FORM_TOKEN_BUDGET, TOKENS_PER_CHAR, BUDGET_CUT

_T = {}
def _traj(tid):
    if not _T:
        import common
        _T.update(common.trajectories())
    return _T.get(tid)
def qkey(q): return hashlib.sha256(q.encode()).hexdigest()[:20]


@register_memory
class G1SpanMemory(Memory):
    memory_type = "warm_g1_span"
    DEFAULTS = {"mode": "span_v", "spans": None, "first": None, "screens_root": None, "images": True, "form_backend": "engine", "store_root": None}

    def __init__(self, memory_params):
        super().__init__(memory_params)
        self.p = {**self.DEFAULTS, **memory_params}
        if self.p["form_backend"] not in ("engine", "reference"): raise ValueError("form_backend must be engine or reference")
        self.trajs = {}; self._s = None; self._f = None; self._engine = None

    def engine(self):
        if self._engine is None: self._engine = formtable.EngineForms(self.p["store_root"])
        return self._engine

    def insert(self, trajectory):
        self.trajs[trajectory["id"]] = trajectory
        if self.p["form_backend"] == "engine": self.engine().ingest(trajectory)

    def form_blocks(self, valid):
        states = [(t, st) for t, a, b in valid for st in t["states"][a:b + 1]]
        if self.p["form_backend"] == "reference": return formtable.reference_blocks([st["accessibility_tree"] or "" for _, st in states])
        for t, _, _ in valid: self.engine().ingest(t)
        return self.engine().blocks([(t["id"], int(st["state_index"])) for t, st in states])

    def query(self, query, query_image=None):
        if self._s is None:
            self._s = json.loads(Path(self.p["spans"]).read_text()) if self.p["spans"] and Path(self.p["spans"]).exists() else {}
        rec = self._s.get(qkey(query)) or {}
        items = []
        if self.p["first"]:
            if self._f is None: self._f = json.loads(Path(self.p["first"]).read_text())
            ft = self._f.get(qkey(query))
            if ft: items.append({"type": "text", "value": ft.rstrip() + "\n"})
        an = (rec.get("analysis") or "").strip()
        if self.p["mode"] in ("span_a", "span_v") and an:
            items.append({"type": "text", "value": "## Support Analysis\n" + an + "\n"})
        sup = rec.get("support")
        if self.p["mode"] == "span_v" and sup:
            items.append({"type": "text", "value": "## Support Verdict\n" + SUPPORT_VERDICT.get(sup, sup) + "\n"})
        spans = rec.get("spans") or []; budget = MAX_TOTAL_SPAN_STATES; valid = []
        for sp in spans:
            t = self.trajs.get(sp["trajectory_id"]) or _traj(sp["trajectory_id"])
            if t is None: continue
            a, b = int(sp["start_state_index"]), int(sp["end_state_index"])
            if a < 0 or b < a or b >= len(t["states"]) or b - a + 1 > budget: continue
            budget -= b - a + 1; valid.append((t, a, b))
        if valid:
            items.append({"type": "text", "value": "## Trajectory State Spans\n" + "\n".join(f"- {t['id']}: states {a}-{b}" for t, a, b in valid) + "\n"})
            items.append({"type": "text", "value": "## Linked Evidence\n"})
        blocks = self.form_blocks(valid); k = 0
        tokb = FORM_TOKEN_BUDGET; used = 0.0; first = True
        for idx, (t, a, b) in enumerate(valid, start=1):
            tj = {**t, "actions": [s["action"] for s in t["states"] if isinstance(s.get("action"), str) and s["action"].strip()]}
            items.append({"type": "text", "value": format_span_header(idx, tj, a, b)})
            for st in t["states"][a:b + 1]:
                body = st["accessibility_tree"] or ""; pre = blocks[k]; k += 1
                est = TOKENS_PER_CHAR * (len(body) + len(pre))
                if not first and used + est > tokb: continue
                if first and est > tokb: body = body[: max(0, int(tokb / TOKENS_PER_CHAR) - len(pre))] + BUDGET_CUT
                used += TOKENS_PER_CHAR * (len(body) + len(pre)); first = False
                s2 = {"state_index": st["state_index"], "step": st.get("step"), "url": st["url"], "action": st.get("action"), "text": body}
                txt = format_state_text(s2, "both")
                if pre: txt = txt.replace("- AXTree:", pre + "- AXTree:", 1)
                items.append({"type": "text", "value": txt})
                shot = st.get("screenshot")
                if self.p["images"] and shot and self.p["screens_root"]:
                    f = Path(self.p["screens_root"]) / t["id"] / Path(shot).name
                    if f.exists(): items.append({"type": "image", "value": str(f)})
        return items
