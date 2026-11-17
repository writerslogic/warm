# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""formstate: generic structured form state from a BrowserGym-style accessibility tree (no site-specific strings).
Controls: textbox, searchbox, combobox, listbox, checkbox, radio, switch, spinbutton, button, tab; properties value, selected, checked,
required, disabled, expanded; options of a combobox/listbox (indented option children; selected ones marked *)."""
import re
CTRL = {"textbox", "searchbox", "combobox", "listbox", "checkbox", "radio", "switch", "spinbutton", "button", "tab", "menuitemcheckbox", "menuitemradio"}
LINE = re.compile(r"^(\s*)\[(\w+)\]\s+(\w+)\s*(?:'((?:[^'\\]|\\.)*)')?(.*)$")
PROP = re.compile(r"(\w+)=('(?:[^'\\]|\\.)*'|[^,]+)")
def parse(text):
    out = []
    for ln in (text or "").split("\n"):
        m = LINE.match(ln)
        if not m: continue
        ind, bid, role, name, rest = m.groups()
        props = {k: v.strip("'") for k, v in PROP.findall(rest or "")}
        flags = {f.strip() for f in (rest or "").split(",") if f.strip() and "=" not in f}
        out.append((len(ind), bid, role, (name or "").strip(), props, flags))
    return out
def table(text, max_opts=25, max_rows=120):
    nodes = parse(text); rows = []; i = 0
    while i < len(nodes) and len(rows) < max_rows:
        d, bid, role, name, p, fl = nodes[i]
        if role in CTRL:
            st = []
            if "value" in p and p["value"] not in ("", "None"): st.append(f"value={p['value']!r}")
            for k in ("checked", "selected", "expanded", "pressed"):
                if k in p: st.append(f"{k}={p[k]}")
            for k in ("required", "disabled", "readonly"):
                if k in p or k in fl: st.append(k if p.get(k, "True") not in ("False", "false") else f"not {k}")
            opts = []
            if role in ("combobox", "listbox"):
                j = i + 1
                while j < len(nodes) and nodes[j][0] > d:
                    if nodes[j][2] == "option": opts.append(("*" if nodes[j][4].get("selected") in ("True", "true") else "") + nodes[j][3])
                    j += 1
            if role == "button" and not st and not opts: i += 1; rows.append(f"button {name!r}") if name else None; continue
            r = f"{role} {name!r}" + (" " + ", ".join(st) if st else "")
            if opts: r += " options: " + " | ".join(opts[:max_opts]) + (f" | (+{len(opts)-max_opts} more)" if len(opts) > max_opts else "")
            rows.append(r)
        i += 1
    return rows
