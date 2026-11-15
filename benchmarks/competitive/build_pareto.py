#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Build-time / serving Pareto table of WARM build profiles against hnswlib builds.

Input: a campaign directory with, per dataset, `camp_<ds>.txt` (BUILD / HBUILD lines) and the serve
JSONs `w_<ds>_<profile>.serve.json` (public-bench ann-qgraph --load-index --repeats 10) and
`h_<ds>_<build>.serve.json` (hnswlib native driver, 10 timed rounds per ef; hs.py).
Output: the result JSON and the markdown tables.

Rules, fixed before the numbers were seen:
  * a WARM profile is "no slower to build" than an hnswlib build when its median build seconds over
    the repeats is at most the hnswlib median;
  * QPS at a recall target is read off the upper envelope of the (recall, QPS) sweep points, log-linear
    between the bracketing points; the interval is the min and max over the 10 timed rounds;
  * a cell dominates when the WARM lower bound exceeds the hnswlib upper bound (or hnswlib never
    reaches the recall and WARM does); otherwise it is reported as not dominating.
"""
import json, math, re, statistics, sys
from pathlib import Path

TARGETS = (0.90, 0.95, 0.99)
PROFILES = ("fast", "balanced", "high", "max-recall")
HNSW = {"m16e200": "M=16 efC=200", "m32e200": "M=32 efC=200", "m32e500": "M=32 efC=500", "m48e500": "M=48 efC=500"}


def envelope(points):
    """[(recall, qps)] -> monotone envelope: best QPS among points of at least this recall."""
    pts = sorted(points)
    out, best = [], 0.0
    for r, q in reversed(pts):
        best = max(best, q)
        out.append((r, best))
    return sorted(out)


def qps_at(points, t):
    env = envelope(points)
    if not env or env[-1][0] < t:
        return None
    lo = [p for p in env if p[0] < t]
    hi = [p for p in env if p[0] >= t]
    h = hi[0]
    if not lo or h[0] == t:
        return h[1]
    l = lo[-1]
    f = (t - l[0]) / (h[0] - l[0])
    return math.exp((1 - f) * math.log(l[1]) + f * math.log(h[1]))


def rounds(path):
    """Per-round QPS at each target: {target: [qps per round]} (None when unreachable)."""
    rows = json.load(open(path))["sweep"]
    n = min(len(r["qps_runs"]) for r in rows)
    out = {t: [] for t in TARGETS}
    for k in range(n):
        pts = [(r["recall_at_10"], r["qps_runs"][k]) for r in rows]
        for t in TARGETS:
            out[t].append(qps_at(pts, t))
    return out


def summarize(vals):
    if any(v is None for v in vals):
        return None
    return {"median": statistics.median(vals), "min": min(vals), "max": max(vals)}


def parse_builds(path):
    w, h = {}, {}
    for line in Path(path).read_text().splitlines():
        m = re.match(r"BUILD rep=(\d+) prof=(\S+) .*?build_s=([\d.]+) hash=(\w+) idx=(\d+) peakKB=(\d+)", line)
        if m:
            w.setdefault(m[2], []).append({"build_s": float(m[3]), "hash": m[4], "index_bytes": int(m[5]), "peak_kb": int(m[6])})
        m = re.match(r"HBUILD rep=(\d+) h_\w+?_(\w+) M=\d+ efc=\d+ th=\d+ build_s=([\d.]+) peakKB=(\d+) idx_bytes=(\d+)", line)
        if m:
            h.setdefault(m[2], []).append({"build_s": float(m[3]), "peak_kb": int(m[4]), "index_bytes": int(m[5])})
    return w, h


def parse_serve_rss(path):
    """Peak RSS (KiB) of each WARM serving process, from the last successful SERVE line per tag."""
    out = {}
    for line in Path(path).read_text().splitlines():
        m = re.match(r"SERVE (\S+) rc=0 srss_kb=(\d+)", line)
        if m:
            out[m[1]] = int(m[2])
    return out


def stats(rows, key):
    v = [r[key] for r in rows]
    return {"median": statistics.median(v), "min": min(v), "max": max(v), "n": len(v)}


def main(directory, out_json, out_md):
    d = Path(directory)
    result, md = {"description": "", "datasets": {}}, []
    for ds in ("sift", "glove"):
        camp = d / f"camp_{ds}.txt"
        if not camp.exists():
            continue
        wb, hb = parse_builds(camp)
        srss = parse_serve_rss(camp)
        entry = {"warm": {}, "hnswlib": {}}
        for p in PROFILES:
            sj = d / f"w_{ds}_{p}.serve.json"
            if p in wb and sj.exists():
                srv = json.load(open(sj))
                entry["warm"][p] = {
                    "build_s": stats(wb[p], "build_s"), "build_peak_kb": stats(wb[p], "peak_kb"),
                    "index_bytes": wb[p][0]["index_bytes"], "graph_hashes": sorted({r["hash"] for r in wb[p]}),
                    "serve_peak_kb": srss.get(f"w_{ds}_{p}"),
                    "qps": {str(t): summarize(v) for t, v in rounds(sj).items()},
                    "sweep": [{k: r[k] for k in ("recall_at_10", "qps_single_thread", "params")} for r in srv["sweep"]],
                }
        for n in HNSW:
            sj = d / f"h_{ds}_{n}.serve.json"
            if n in hb and sj.exists():
                srv = json.load(open(sj))
                entry["hnswlib"][n] = {
                    "build_s": stats(hb[n], "build_s"), "build_peak_kb": stats(hb[n], "peak_kb"),
                    "index_bytes": hb[n][0]["index_bytes"], "serve_peak_kb": srv.get("serve_peak_kb"),
                    "qps": {str(t): summarize(v) for t, v in rounds(sj).items()},
                    "sweep": [{k: r[k] for k in ("ef", "recall_at_10", "qps_single_thread")} for r in srv["sweep"]],
                }
        rows = []
        for n, hv in entry["hnswlib"].items():
            ok = [p for p, wv in entry["warm"].items() if wv["build_s"]["median"] <= hv["build_s"]["median"]]
            fastest = min(ok, key=lambda p: entry["warm"][p]["build_s"]["median"]) if ok else None

            def cell(p):
                if p is None:
                    return {"profile": None}
                c = {"profile": p, "build_s_median": entry["warm"][p]["build_s"]["median"], "ratios": {}}
                for t in TARGETS:
                    w, h = entry["warm"][p]["qps"][str(t)], hv["qps"][str(t)]
                    if w is None and h is None:
                        c["ratios"][str(t)] = {"verdict": "tie: neither reaches the recall"}
                    elif w is None:
                        c["ratios"][str(t)] = {"verdict": "WARM does not reach the recall"}
                    elif h is None:
                        c["ratios"][str(t)] = {"verdict": "dominates (hnswlib does not reach the recall)", "warm_qps": w["median"]}
                    else:
                        lo, hi = w["min"] / h["max"], w["max"] / h["min"]
                        c["ratios"][str(t)] = {"ratio": w["median"] / h["median"], "low": lo, "high": hi,
                                               "verdict": "dominates" if lo > 1 else "does not dominate"}
                return c

            best = max(ok, key=lambda p: (entry["warm"][p]["qps"]["0.99"] or {"median": 0})["median"] + 1e-3 * (entry["warm"][p]["qps"]["0.95"] or {"median": 0})["median"]) if ok else None
            rows.append({"hnswlib": n, "no_slower_profiles": ok, "fastest": cell(fastest), "best_serving": cell(best)})
        entry["table"] = rows
        result["datasets"][ds] = entry

        md.append(f"### {ds}\n")
        md.append("Build seconds (median, min-max over repeats), peak build RSS, index bytes, serving RSS, QPS at recall (median, min-max over 10 rounds, one pinned core).\n")
        md.append("| system | build s | build RSS MiB | index MiB | serve RSS MiB | QPS@0.90 | QPS@0.95 | QPS@0.99 |")
        md.append("|---|---|---|---|---|---|---|---|")

        def fmt(q):
            return "not reached" if q is None else f"{q['median']:,.0f} ({q['min']:,.0f}-{q['max']:,.0f})"

        for kind, name, v in [("WARM", p, entry["warm"][p]) for p in entry["warm"]] + [("hnswlib", n, entry["hnswlib"][n]) for n in entry["hnswlib"]]:
            b = v["build_s"]
            md.append(f"| {kind} {HNSW.get(name, name)} | {b['median']:.1f} ({b['min']:.1f}-{b['max']:.1f}) | {v['build_peak_kb']['median'] / 1024:,.0f} | "
                      f"{v['index_bytes'] / 2**20:,.0f} | {(v['serve_peak_kb'] or 0) / 1024:,.0f} | " + " | ".join(fmt(v["qps"][str(t)]) for t in TARGETS) + " |")
        md.append("\nFor each hnswlib build: the WARM profiles whose median build is no slower, the fastest of them, and the best-serving of them, as WARM/hnswlib QPS ratio (lower-upper bound over rounds).\n")
        md.append("| hnswlib build | no-slower WARM profiles | column | profile | QPS@0.90 | QPS@0.95 | QPS@0.99 |")
        md.append("|---|---|---|---|---|---|---|")
        for r in rows:
            for col in ("fastest", "best_serving"):
                c = r[col]
                if c["profile"] is None:
                    md.append(f"| {HNSW[r['hnswlib']]} | none | {col} | none | - | - | - |")
                    continue
                cells = []
                for t in TARGETS:
                    x = c["ratios"][str(t)]
                    cells.append(f"{x['ratio']:.2f}x ({x['low']:.2f}-{x['high']:.2f}) {x['verdict']}" if "ratio" in x else x["verdict"])
                md.append(f"| {HNSW[r['hnswlib']]} | {', '.join(r['no_slower_profiles'])} | {col} | {c['profile']} | " + " | ".join(cells) + " |")
        md.append("")
    Path(out_json).write_text(json.dumps(result, indent=1))
    Path(out_md).write_text("\n".join(md))


if __name__ == "__main__":
    main(*sys.argv[1:4])
