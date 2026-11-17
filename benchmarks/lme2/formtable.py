# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Form-control table blocks for the G1 renderer.

reference: formstate.table over the page text, rows already shown earlier in the same query omitted.
engine:    the WARM engine (writerslogic_warm.Warm with experience_form_state) builds the same blocks from
           trajectory states ingested once at Memory.insert time."""
import re, tempfile
import formstate
from prompts import form_block

MAX_STR = 16384
MAX_AXTREE = 4 << 20
MAX_NODES = 200000
MAX_DEPTH = 128
# Same handle, role and quoted-name grammar as formstate.LINE, with the handle optional so every line becomes a node.
LINE = re.compile(r"^(\s*)(?:\[(\w+)\]\s+)?(\w+)\s*(?:'((?:[^'\\]|\\.)*)')?(.*)$")


def clip(s, n=MAX_STR):
    b = s.encode()
    return s if len(b) <= n else b[:n].decode("utf-8", "ignore")


def build_tree(text):
    """Accessibility-tree text to {"role", "name", "props"?, "handle"?, "children"?}; a line nests under the nearest
    earlier line with less leading whitespace. Several top-level lines get a synthetic RootWebArea parent."""
    tops = []; stack = []; count = 0
    for ln in (text or "").split("\n"):
        m = LINE.match(ln)
        if not m: continue
        if count >= MAX_NODES - 1: break
        ind, bid, role, name, rest = m.groups()
        node = {"role": clip(role), "name": clip((name or "").strip())}
        if rest: node["props"] = clip(rest)
        if bid: node["handle"] = clip(bid)
        d = len(ind)
        while stack and stack[-1][0] >= d: stack.pop()
        while len(stack) > MAX_DEPTH - 2: stack.pop()
        (stack[-1][1].setdefault("children", []) if stack else tops).append(node)
        stack.append((d, node)); count += 1
    if len(tops) == 1: return tops[0]
    root = {"role": "RootWebArea", "name": ""}
    if tops: root["children"] = tops
    return root


def reference_blocks(texts):
    shown_rows = set(); out = []
    for raw in texts:
        rows = formstate.table(raw); new = [r for r in rows if r not in shown_rows]; shown_rows.update(rows)
        out.append(form_block(new, len(rows)) if rows else "")
    return out


def state_event(t, k, st):
    ev = {"episode": t["id"], "step": int(st["state_index"]), "tree": build_tree(st["accessibility_tree"] or ""), "url": clip(str(st["url"] or ""))}
    act = st.get("action")
    if k > 0 and isinstance(act, str) and act.strip(): ev["action"] = {"call": clip(act)}
    ev["accessibilityTree"] = clip(st["accessibility_tree"] or "", MAX_AXTREE)
    return ev


class EngineForms:
    def __init__(self, store_root=None):
        import writerslogic_warm
        self.w = writerslogic_warm.Warm(path=tempfile.mkdtemp(prefix="g1-forms-", dir=store_root), experience_form_state=True)
        self.ingested = set()

    def ingest(self, t):
        if t["id"] in self.ingested: return
        for k, st in enumerate(t["states"]): self.w.remember(state_event(t, k, st))
        self.ingested.add(t["id"])

    def blocks(self, keys):
        out = list(self.w.experience_form_tables(keys))
        if len(out) != len(keys): raise RuntimeError(f"engine returned {len(out)} form blocks for {len(keys)} states")
        return out
