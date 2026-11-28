# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Tables for the JavaScript comparison (`warm-bench js`): WARM against Vectra, both called in-process from Node.

Reads `kind: js` results files. A metric is one observation per process round (or build repeat); WARM's factor against
Vectra is the ratio of medians (rounds are paired: round i ran both rows), oriented so that more than 1 is WARM ahead,
and a verdict needs a percentile bootstrap interval of that factor that excludes 1 with at least `analyze.MIN_BOOTSTRAP_ROUNDS`
observations a side. Quality is a paired per-query bootstrap of nDCG@10.
"""

import json
import statistics

from . import analyze, common
from .table import RESULTS, fx, rel
from .table import fmt as _fmt


def fmt(value, decimals):
    """Table formatting; a sub-10 value keeps two decimals so a slow system does not print as 0."""
    return _fmt(value, 2 if decimals == 0 and value is not None and abs(value) < 10 else decimals)

WARM_FOR = {"dense": ("warm-vector", "vectra"), "hybrid": ("warm-hybrid", "vectra-bm25")}
CONTRACT_TITLE = {
    "dense": "Dense exact cosine, top 10 (vector records on both sides)",
    "hybrid": "Hybrid: WARM fused BM25 + cosine against Vectra's `isBm25` option",
    "keyword": "Keyword only",
}
# metric key -> (column label, higher is better, decimals)
COLUMNS = [("ndcg", "nDCG@10", True, 4), ("qps", "QPS", True, 0), ("p50", "p50 (us)", False, 0), ("p99", "p99 (us)", False, 0),
           ("build", "Build (s)", False, 2), ("size", "Index (MiB)", False, 1), ("rss", "Serve RSS (MiB)", False, 0), ("cold", "Cold start (s)", False, 2)]


def load():
    found = {}
    for path in sorted(RESULTS.glob("*-js-*.json")):
        data = json.loads(path.read_text())
        if data.get("schema_version") != 1 or data.get("kind") != "js" or data.get("suite") == "smoke":
            continue
        key = (data["dataset"]["id"], data["hardware"]["class"], data["suite"])
        if key not in found or data["date"] > found[key][1]["date"]:
            found[key] = (path, data)
    return sorted(found.values(), key=lambda pd: (pd[1]["dataset"]["name"], pd[1]["hardware"]["class"]))


def per_round(entry, field):
    """One value per serve round: median over the round's timed passes."""
    rounds = entry["serve_rounds"]
    if field == "qps":
        return [statistics.median(p["queries"] / (p["total_ns"] / 1e9) for p in r["passes"]) for r in rounds]
    if field in ("p50", "p99"):
        return [statistics.median(p[f"{field}_ns"] for p in r["passes"]) / 1e3 for r in rounds]
    if field == "cold":
        return [r["cold_start_seconds"] for r in rounds]
    if field == "rss":
        return [r["peak_rss_bytes"] / 2**20 for r in rounds]
    if field == "build":
        return [b["build_seconds"] for b in entry["build_runs"]]
    if field == "size":
        return [b["index_bytes"] / 2**20 for b in entry["build_runs"]]
    raise KeyError(field)


def value(entry, field, subset=None):
    if field == "ndcg":
        per = entry["quality"]["per_query_ndcg@10"]
        return statistics.fmean(v for q, v in per.items() if subset is None or q in subset)
    return statistics.median(per_round(entry, field))


def advantage(warm, other, field, higher):
    """(factor, interval, observations): more than 1 is WARM ahead."""
    mine, theirs = per_round(warm, field), per_round(other, field)
    paired = field in ("qps", "p50", "p99", "cold", "rss") and len(mine) == len(theirs)
    num, den = (mine, theirs) if higher else (theirs, mine)
    factor = statistics.median(num) / statistics.median(den)
    ci = common.round_ratio_ci(num, den, paired) if min(len(mine), len(theirs)) > 1 else None
    return factor, ci, min(len(mine), len(theirs))


def mark(factor, ci, n):
    if ci is None or n < analyze.MIN_BOOTSTRAP_ROUNDS:
        return "n/v"
    return "WIN" if ci[0] > 1 else "LOSS" if ci[1] < 1 else "TIE"


def quality_advantage(warm, other):
    a, b = warm["quality"]["per_query_ndcg@10"], other["quality"]["per_query_ndcg@10"]
    shared = sorted(set(a) & set(b))
    result = common.paired_bootstrap([a[q] for q in shared], [b[q] for q in shared])
    lo, hi = result["difference_ci"]
    return result, ("n/v" if len(shared) < 30 else "WIN" if lo > 0 else "LOSS" if hi < 0 else "TIE")


def comparisons(data, contract):
    """Rows of (label, warm value, vectra value, factor text, verdict, source note) for one contract."""
    warm_key, vectra_key = WARM_FOR[contract]
    systems = data["systems"]
    if warm_key not in systems or vectra_key not in systems:
        return []
    warm, other = systems[warm_key], systems[vectra_key]
    subset = set(other["quality"]["per_query_ndcg@10"]) if contract == "hybrid" else None
    out = []
    for key, label, higher, dec in COLUMNS:
        if key == "ndcg":
            result, verdict = quality_advantage(warm, other)
            out.append((key, label, value(warm, key, subset), value(other, key, subset), f"{result['difference']:+.4f} nDCG", verdict, dec))
            continue
        if contract == "hybrid" and key in ("build", "size"):
            continue
        factor, ci, n = advantage(warm, other, key, higher)
        verdict = mark(factor, ci, n)
        out.append((key, label, value(warm, key), value(other, key), fx(factor), verdict, dec))
    return out


def cells_for(path, data):
    """Every table cell of one results file, recomputed from the recorded observations."""
    hw, ds, systems = data["hardware"]["class"], data["dataset"], data["systems"]
    out = []
    for contract in ("dense", "hybrid", "keyword"):
        subset = set(systems["vectra-bm25"]["quality"]["per_query_ndcg@10"]) if contract == "hybrid" and "vectra-bm25" in systems else None
        for key, entry in systems.items():
            if entry["contract"] != contract:
                continue
            for metric, label, higher, dec in COLUMNS:
                if metric in ("build", "size") and contract == "hybrid" and key == "vectra-bm25":
                    continue
                v = value(entry, metric, subset)
                rounds = per_round(entry, metric) if metric != "ndcg" else None
                out.append({"section": f"js {ds['name']} {contract}", "file": rel(path), "hardware_class": hw, "metric": metric, "system": key,
                            "value": v, "ci": common.bootstrap_ci(rounds) if rounds else None,
                            "n": len(rounds) if rounds else entry["quality"]["queries"],
                            "source": f"{rel(path)}#systems.{key}", "validity": "valid", "uncertainty": "sampled",
                            "interval_kind": "bootstrap", "text": fmt(v, dec)})
    return out


def lines(results, cells):
    if not results:
        return []
    out = ["## JavaScript in-process: WARM and Vectra", "",
           "Both stores are called from one Node process (`warm-memory` for WARM, `vectra` for Vectra), index the same 384-dimension document vectors "
           "(WARM's local embedder, one file shared by every row) and answer with the same query vectors, so the comparison is the stores, not the models. "
           "Quality is nDCG@10 and recall@10 of the returned top 10; QPS and latency are one query at a time with the hit ids read inside the timer; "
           "build, size, serve RSS and cold start are per fresh process. Vectra's `isBm25` option is measured on a small query sample because it rebuilds "
           "a BM25 engine per query and exhausts the heap after about 20 queries in one process. Factors are ratios of medians, more than 1x is WARM ahead; "
           "W, T and L follow the same bootstrap rule as the sections above, `n/v` is no verdict. Method, versions and the unsupported rows are recorded in "
           "each results file (`kind: js`); how to reproduce: [index.md](index.md).", ""]
    for path, data in results:
        hw, ds, systems = data["hardware"]["class"], data["dataset"], data["systems"]
        out += [f"### {ds['name']} ({ds['documents']:,} documents, {ds['queries']:,} queries) on `{hw}`", "",
                f"Versions: Node {data['versions']['node']}, vectra {data['versions']['vectra']}, warm-memory {data['versions']['warm-memory']}. "
                f"Embedding: {data['embedding']['model']} {data['embedding']['quantization']}, {data['embedding']['dimensions']} dimensions. "
                f"{data['protocol']['rounds']} rounds, {data['protocol']['build_repeats']} builds, results `{rel(path)}`.", ""]
        cells.extend(cells_for(path, data))
        for contract in ("dense", "hybrid", "keyword"):
            rows = [(k, s) for k, s in systems.items() if s["contract"] == contract]
            if not rows and contract != "keyword":
                continue
            out += [f"**{CONTRACT_TITLE[contract]}**", "", "| System | " + " | ".join(c[1] for c in COLUMNS) + " |", "|---|" + "---|" * len(COLUMNS)]
            subset = set(systems["vectra-bm25"]["quality"]["per_query_ndcg@10"]) if contract == "hybrid" and "vectra-bm25" in systems else None
            for key, entry in rows:
                cell = []
                for metric, label, higher, dec in COLUMNS:
                    if metric in ("build", "size") and contract == "hybrid" and key == "vectra-bm25":
                        cell.append("see dense")
                        continue
                    v = value(entry, metric, subset)
                    cell.append(fmt(v, dec))
                out.append(f"| {entry['display']} | " + " | ".join(cell) + " |")
            for _, unsupported in [(k, v) for k, v in data.get("unsupported", {}).items()]:
                if contract in unsupported:
                    out.append(f"| Vectra 0.15.0 | " + " | ".join(["unsupported"] * len(COLUMNS)) + " |")
            if contract in WARM_FOR:
                rowv = comparisons(data, contract)
                if rowv:
                    by_key = {r[0]: f"{r[4]} {r[5]}" for r in rowv}
                    out.append("| **WARM against Vectra** | " + " | ".join(by_key.get(c[0], "-") for c in COLUMNS) + " |")
            out.append("")
            for _, unsupported in data.get("unsupported", {}).items():
                if contract in unsupported:
                    out += [f"Vectra, {contract}: {unsupported[contract]}", ""]
            if contract == "hybrid" and "vectra-bm25" not in systems:
                out += [f"Vectra, hybrid: not run. {data['not_run']['vectra-bm25']}", ""]
        if "vectra-bm25" in systems and "warm-hybrid" in systems:
            out += [f"The hybrid rows compare nDCG@10 on the same {len(systems['vectra-bm25']['quality']['per_query_ndcg@10'])} queries Vectra's BM25 row ran; "
                    f"WARM hybrid over all {systems['warm-hybrid']['quality']['queries']:,} queries is {systems['warm-hybrid']['quality']['ndcg@10']:.4f}, "
                    f"WARM dense {systems['warm-vector']['quality']['ndcg@10']:.4f}, Vectra dense {systems['vectra']['quality']['ndcg@10']:.4f}. "
                    "Vectra's list is the 10 semantic hits followed by BM25 hits, so its top-10 quality is the dense quality by construction.", ""]
    return out


def where_warm_loses(results):
    rows = []
    for path, data in results:
        for contract in WARM_FOR:
            for key, label, w, v, text, verdict, dec in comparisons(data, contract):
                rows.append((data["dataset"]["name"], data["hardware"]["class"], contract, label, w, v, text, verdict, dec))
    return [r for r in rows if r[7] == "LOSS" or (r[7] == "n/v" and _behind(r))]


def _behind(row):
    text = row[6]
    return text.endswith("x") and float(text[:-1]) < 1 or text.startswith("-")


def loss_lines(results):
    behind = where_warm_loses(results)
    if not behind:
        return ["No JavaScript metric is lost.", ""]
    out = ["### Where WARM loses to Vectra (JavaScript)", "", "| Dataset | Contract | Metric | WARM | Vectra | WARM against Vectra | Verdict |", "|---|---|---|---|---|---|---|"]
    for ds, hw, contract, label, w, v, text, verdict, dec in behind:
        out.append(f"| {ds} (`{hw}`) | {contract} | {label} | {fmt(w, dec)} | {fmt(v, dec)} | {text} | {'LOSS' if verdict == 'LOSS' else 'behind, no verdict'} |")
    return out + [""]


def readme_rows(results):
    """Rows for the README block table: Dataset | Machine | Metric | WARM | Best other | Advantage | mark."""
    marks = {"WIN": "W", "TIE": "T", "LOSS": "L", "n/v": "-"}
    out = []
    for path, data in results:
        ds, hw = data["dataset"]["name"], data["hardware"]["class"]
        for key, label, w, v, text, verdict, dec in comparisons(data, "dense"):
            if key in ("ndcg", "qps", "p50", "build", "size"):
                out.append(f"| {ds} (JS, vs Vectra {data['versions']['vectra']}) | `{hw}` | {label} | {fmt(w, dec)} | Vectra {fmt(v, dec)} | {text} | {marks[verdict]} |")
    return out
