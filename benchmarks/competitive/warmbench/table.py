# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Generate docs/benchmarks/COMPARISON.md, the README block and the cell index.

Reads only the committed benchmarks/results/competitive/*.json (smoke runs excluded). Every
number in the output is produced by warmbench.analyze from those files; docs/benchmarks/
comparison.cells.json lists every cell with the file and key path it came from, and
tests/test_table.py recomputes each one.
"""

import json
import re
import sys
from pathlib import Path

from . import analyze, jars, registry
from .common import RESULTS, ROOT, round_ratio_ci

DOCS = ROOT / "docs/benchmarks"
COMPARISON = DOCS / "COMPARISON.md"
CELLS = DOCS / "comparison.cells.json"
BLOCK = ROOT / "benchmarks/competitive/README-block.md"
README = ROOT / "README.md"
START, END = "<!-- BENCH:START -->", "<!-- BENCH:END -->"

# metric key -> (label, unit label, decimals). Order is the display order.
ANN_METRICS = [
    ("qps@recall0.90", "QPS @ recall@10 >= 0.90", "QPS", 0), ("qps@recall0.95", "QPS @ recall@10 >= 0.95", "QPS", 0),
    ("qps@recall0.99", "QPS @ recall@10 >= 0.99", "QPS", 0),
    ("p50_latency@recall0.95", "p50 latency @ 0.95", "us", 0), ("p99_latency@recall0.95", "p99 latency @ 0.95", "us", 0),
    ("build_seconds", "Build time", "s", 2), ("build_peak_rss_mib", "Peak RSS, build", "MiB", 0),
    ("serving_rss_mib", "RSS, serving", "MiB", 0), ("index_bytes_mib", "Index size", "MiB", 1),
    ("cold_start_s", "Cold start to first answer", "s", 2),
    ("replace_ms_per_op", "Replace one vector (durable)", "ms", 3), ("delete_ms_per_op", "Delete one vector (durable)", "ms", 3),
    ("install_mib", "Install size", "MiB", 1), ("install_distributions", "Install distributions", "count", 0),
]
TEXT_METRICS = [
    ("ndcg@10", "nDCG@10", "", 4), ("recall@100", "recall@100", "", 4),
    ("qps", "QPS (single thread)", "QPS", 0), ("p50_latency", "p50 latency", "us", 0), ("p99_latency", "p99 latency", "us", 0),
    ("build_seconds", "Ingest/build time", "s", 2), ("build_peak_rss_mib", "Peak RSS, build", "MiB", 0),
    ("serving_rss_mib", "Peak RSS, serving", "MiB", 0), ("disk_mib", "On-disk size", "MiB", 1),
    ("cold_start_s", "Cold start to first answer", "s", 2),
    ("replace_ms_per_op", "Replace one document (durable)", "ms", 3), ("delete_ms_per_op", "Delete one document (durable)", "ms", 3),
    ("install_mib", "Install size", "MiB", 1), ("install_distributions", "Install distributions", "count", 0),
]
HEADLINE = {
    "ann": ["qps@recall0.95", "p99_latency@recall0.95", "build_seconds", "serving_rss_mib", "index_bytes_mib"],
    "text": ["ndcg@10", "qps", "p99_latency", "build_seconds", "serving_rss_mib", "disk_mib"],
}

# Candidate levers per metric. These are hypotheses for the coordinator, not measurements, except
# where a results file is named.
LEVERS = {
    "text:build_seconds": "hypothesis (profiles not committed): ingest is tokenizing and stemming, the serial postings install (postings::index_chunks), durable fsyncs and the checkpoint written at close; open levers are a parallel postings install and fewer syncs per batch",
    "text:build_peak_rss_mib": "hypothesis (allocator traces not committed): the prepared documents and the postings install are live together within a batch and the input JSON is held whole; smaller batches would lower the peak at the cost of more durable commits",
    "text:cold_start_s": "the store is read into memory at open (open_s in the raw file); lazy or memory-mapped loading of the arena and postings is the lever",
    "text:serving_rss_mib": "the whole store is resident after open; memory-mapped postings and file-backed text (ledger 2A/2B) are the open levers",
    "text:p99_latency": "tail queries are the long ones; MaxScore-style early termination for long queries (the ledger's impact-index work, category 1C) is the candidate",
    "text:qps": "hypothesis (profiles not committed): posting decoding from the mapped checkpoint and the exhaustive rank pass dominate long queries; per-hit record decoding matters on small corpora; a block-packed checkpoint posting encoding and early termination are the candidates",
    "text:p50_latency": "same as QPS: posting decoding from the mapped checkpoint and per-hit record decoding",
    "text:disk_mib": "compact postings and file-backed text (ledger 2B/2C) and removing preallocated arena slack",
    "text:recall@100": "none found: the paired per-query difference is within its interval on every corpus; a development-set search over BM25 k1, b and the idf form (benchmarks/results/lexical_ranking_selection.json) gave a default that raised some reported cells and lowered others, so it was not adopted (benchmarks/results/lexical_ranking_evaluation.json)",
    "text:replace_ms_per_op": "batching commits (memorize_batch) or a group commit; the per-op figure is one durable commit",
    "text:delete_ms_per_op": "batching commits (memorize_batch) or a group commit; the per-op figure is one durable commit",
    "qps@recall0.90": "per-query estimate count and memory traffic of the vertex graph (the vertex-graph rows under benchmarks/results)",
    "qps@recall0.95": "per-query estimate count and memory traffic of the vertex graph (ledger 1A; the neighbour-block fast-scan lever was killed on M4 in benchmarks/results/qgraph_block_scan_microbench.json, so the open levers are pruning and an x86 kernel)",
    "qps@recall0.99": "higher-recall operating points need more re-rank candidates; a cheaper re-rank (residual 4-bit, benchmarks/results/qgraph_bytes_paired_m4_glove_v1.json) or exact re-rank of fewer candidates",
    "p50_latency@recall0.95": "same as QPS at 0.95 (latency is the inverse of the single-thread rate here)",
    "p99_latency@recall0.95": "same as QPS at 0.95; tail latency comes from the slowest queries' expansion counts",
    "build_seconds": "graph construction cost: parallel build with a cheaper construction search (quantized_search) and a smaller build_ef, at a recall cost to be measured; for text, a lexical-only ingest path that skips dense-slot encoding",
    "build_peak_rss_mib": "stream the build instead of holding the f32 matrix plus graph (ledger 1A target: peak build RSS <= 2x final index bytes)",
    "serving_rss_mib": "persist the index memory-mapped and drop the build-time allocations; for text the serving V40 RSS campaign in the ledger (2A) is the open lever",
    "index_bytes_mib": "4-bit residual codes shrink the index 26% (benchmarks/results/qgraph_bytes_paired_m4_glove_v1.json) at a measured QPS cost; coarser id coding",
    "disk_mib": "compact postings and file-backed text (ledger 2B/2C work) and removing preallocated arena slack",
    "cold_start_s": "ANN: persist and mmap the VGraph (no persisted ANN index exists today); text: lazy-load the arena instead of reading it at open",
    "replace_ms_per_op": "batching commits (memorize_batch) or a group-commit log; the per-op figure is one durable commit",
    "delete_ms_per_op": "batching commits or a group-commit log",
    "install_mib": "strip symbols, split optional features out of the default binary",
    "install_distributions": "none needed",
}


# Display names of dataset ids whose id alone misleads. `memory` is a lexical search-speed corpus built from LongMemEval_S
# dev sessions, not a memory-QA benchmark; the id stays, since committed results key on it.
DATASET_LABELS = {"memory": "LongMemEval-dev session corpus (search speed only, not memory QA)"}


def dataset_label(ds):
    key = ds.get("id") or ds["name"]
    return DATASET_LABELS.get(key, key)


def fmt(value, decimals):
    if value is None:
        return "n/m"
    if decimals == 0:
        return f"{value:,.0f}"
    return f"{value:,.{decimals}f}"


def load_results():
    """(path, data) for every committed non-smoke results file, newest per key."""
    found = {}
    for path in sorted(RESULTS.glob("*.json")):
        if path.name.startswith("smoke-"):
            continue
        data = json.loads(path.read_text())
        if data.get("schema_version") != 1 or data.get("kind") not in ("ann", "text"):
            continue
        if data["kind"] == "text" and not (data.get("protocol") or {}).get("contracts"):
            # Written before the ids/documents timing contracts: WARM and the comparators were timed on different
            # boundaries, so the rows contradict the current protocol. The file stays as history, out of the tables.
            continue
        key = (data["kind"], data["dataset"].get("id") or data["dataset"].get("name"), data["dataset"].get("n_train"), data["hardware"]["class"], data["suite"],
               data.get("mode", "embedded"))
        if key not in found or data["date"] > found[key][1]["date"]:
            found[key] = (path, data)
    views = [view for path, data in found.values() for view in contract_views(path, data)]
    return sorted(views, key=lambda pd: (pd[1]["kind"], str(pd[1]["dataset"]), pd[1]["suite"], pd[1]["hardware"]["class"], pd[1].get("contract", "")))


def contract_views(path, data):
    """A text results file written under two timing contracts is read as one view per contract: the systems of that
    contract and its WARM reference row (`protocol.contracts`). The cell source paths keep the file's own system keys.
    A file without contracts is one view, unchanged."""
    contracts = (data.get("protocol") or {}).get("contracts")
    if data["kind"] != "text" or not contracts:
        return [(path, data)]
    views = []
    for name, spec in contracts.items():
        members = {s: v for s, v in data["systems"].items() if v.get("contract", registry.contract(s)) == name}
        if spec["reference"] in members:
            views.append((path, {**data, "systems": members, "reference": spec["reference"], "contract": name}))
    return views


def display(system, data=None):
    """Name of a system as measured: Lucene carries the version recorded in the result file, never the current pin."""
    name = registry.system(system)["display"]
    if system in ("lucene", "lucene-docs", "lucene-dv-docs"):
        recorded = (((data or {}).get("systems") or {}).get(system) or {}).get("version") or {}
        if recorded.get("lucene"):
            return name.replace(f"Lucene {jars.VERSION}", f"Lucene {recorded['lucene']}")
        return name.replace(f" {jars.VERSION}", "")
    return name


def rel(path):
    try:
        return str(Path(path).resolve().relative_to(ROOT))
    except ValueError:
        return str(Path(path).resolve())


# Interval kinds of an advantage factor. Only the bootstrap kinds can carry a verdict.
BOOTSTRAP_KINDS = ("paired bootstrap", "bootstrap", "paired per-query bootstrap")


def interval_label(adv):
    """What the bracket printed after a factor is, in words."""
    kind = adv.get("interval_kind")
    if kind == "paired per-query bootstrap":
        return f"paired per-query bootstrap, {adv['paired']['queries']} queries"
    if kind == "paired bootstrap":
        return f"paired bootstrap, {adv['observations'][0]} rounds"
    if kind == "bootstrap":
        return f"bootstrap, {adv['observations'][0]} and {adv['observations'][1]} observations"
    if kind == "range":
        return f"min-max range, {adv['observations'][0]} and {adv['observations'][1]} observations: too few for a verdict"
    return ""


def adv_interval_text(adv):
    if not adv or not adv.get("ci"):
        return ""
    return f" [{adv['ci'][0]:.2f}, {adv['ci'][1]:.2f}] ({interval_label(adv)})"


def _sampled_factor(mine, theirs, higher, point):
    """Interval of the advantage factor from the two cells' per-observation values. At least MIN_BOOTSTRAP_ROUNDS
    observations on each side: a percentile bootstrap of the ratio of medians at the cell level, resampling rounds in
    pairs when both cells come from the same interleaved rounds and independently otherwise. Fewer: the min-max range
    (of the per-round ratios when paired), which carries no verdict."""
    a, b = mine["rounds"], theirs["rounds"]
    num, den = (a, b) if higher else (b, a)
    paired = mine.get("pairing") is not None and mine.get("pairing") == theirs.get("pairing") and len(a) == len(b)
    level = min(mine.get("level", 0.95), theirs.get("level", 0.95))
    out = {"factor": point, "observations": [len(a), len(b)]}
    if min(len(a), len(b)) >= analyze.MIN_BOOTSTRAP_ROUNDS:
        ci = round_ratio_ci(num, den, paired, level=level)
        return out | {"ci": ci, "interval_kind": "paired bootstrap" if paired else "bootstrap", "level": level}
    ratios = [x / y for x, y in zip(num, den)] if paired else [x / y for x in num for y in den]
    return out | {"ci": [min(ratios), max(ratios)], "interval_kind": "range"}


def advantage(metric, mine, theirs):
    """WARM advantage factor (>1 means WARM is better) with the interval of the factor itself, or None.
    An unverified side keeps its measured factor with `unverified` set: the ratio is printed, no verdict is drawn."""
    if mine["value"] is None or theirs["value"] is None:
        return None
    if any(c.get("validity", "valid") != "valid" for c in (mine, theirs)):
        if metric in analyze.OPS_METRICS and mine.get("durability_class") != theirs.get("durability_class"):
            return {"factor": None, "ci": None, "unverified": True}
        adv = advantage(metric, {**mine, "validity": "valid"}, {**theirs, "validity": "valid"})
        return {"factor": None, "ci": None, "unverified": True} if adv is None else adv | {"unverified": True}
    if analyze.comparison_note(metric, mine, theirs):
        return None
    higher = analyze.direction(metric) == analyze.HIGHER
    if mine["value"] <= 0 or theirs["value"] <= 0:
        if mine["value"] == theirs["value"]:
            return {"factor": 1.0, "ci": None, "deterministic": mine.get("uncertainty") == theirs.get("uncertainty") == "deterministic"}
        better = (mine["value"] > theirs["value"]) == higher
        return {"factor": float("inf") if better else 0.0, "ci": None,
                "deterministic": mine.get("uncertainty") == theirs.get("uncertainty") == "deterministic"}
    if metric in analyze.PAIRED and mine.get("per_query") and theirs.get("per_query"):
        adv = analyze.paired_advantage(metric, mine, theirs)
        return adv | {"interval_kind": "paired per-query bootstrap" if adv["ci"] else None}
    point = mine["value"] / theirs["value"] if higher else theirs["value"] / mine["value"]
    if mine.get("rounds") and theirs.get("rounds") and min(len(mine["rounds"]), len(theirs["rounds"])) > 1:
        if min(mine["rounds"] + theirs["rounds"]) <= 0:
            return {"factor": point, "ci": None}
        return _sampled_factor(mine, theirs, higher, point)
    return {"factor": point, "ci": None, "deterministic": mine.get("uncertainty") == theirs.get("uncertainty") == "deterministic"}


def fx(factor):
    if factor is None:
        return "n/v"
    if factor == float("inf"):
        return "inf"
    return f"{factor:.2f}x"


def verdict(adv):
    """WIN or LOSS only when a bootstrap interval of the factor excludes 1 (each side with at least
    analyze.MIN_BOOTSTRAP_ROUNDS observations, or a paired per-query bootstrap); a min-max range, a missing interval or
    an unverified side gives n/v; deterministic byte counts use a 5 percent band."""
    if adv is None:
        return "n/m"
    if adv.get("unverified"):
        return "n/v"
    if adv["ci"]:
        if adv.get("interval_kind") not in BOOTSTRAP_KINDS:
            return "n/v"
        return "WIN" if adv["ci"][0] > 1 else "LOSS" if adv["ci"][1] < 1 else "TIE"
    if not adv.get("deterministic", False):
        return "n/v"
    return "WIN" if adv["factor"] > 1.05 else "LOSS" if adv["factor"] < 1 / 1.05 else "TIE"


def best_other(metric, cells, same_contract_as=None):
    """Strongest comparator for `metric`. For a durable-operation row pass WARM's cell as `same_contract_as`: only
    systems with the same commit contract (registry.COMMIT_CONTRACT) are eligible."""
    sign = 1 if analyze.direction(metric) == analyze.HIGHER else -1
    # The eager WARM row is reported, never a comparator.
    others = {s: c for s, c in cells.items()
              if registry.system(s)["family"] != "warm" and c["value"] is not None and not c.get("informational")
              and (same_contract_as is None or c.get("durability_class") == same_contract_as.get("durability_class"))}
    if not others:
        return None, None
    best = max(others, key=lambda s: sign * others[s]["value"])
    return best, others[best]


def reference(data):
    """The WARM row verdicts are drawn for: the library, or warm-server in a server-mode file."""
    return data.get("reference", registry.WARM if registry.WARM in data["systems"] else "hms")


def is_server(data):
    return data.get("mode") == "server"


def _mark_validity(data, metrics):
    revision = data.get("commit", "")
    pinned = bool(re.fullmatch(r"[0-9a-fA-F]{40}", revision))
    if (data.get("source") or {}).get("draft"):
        for systems in metrics.values():
            for c in systems.values():
                c.setdefault("unverified_reason", "draft result: " + ("; ".join(data["source"].get("draft_reasons") or []) or "marked WARM_BENCH_DRAFT"))
    for metric, systems in metrics.items():
        status = data.get("measurement_validity", {}).get(metric, {})
        for c in systems.values():
            validity = analyze.Validity(status.get("status", "valid"))
            reason = status.get("reason", "") or ("" if pinned else "source revision is not a full 40-character commit id") or c.get("unverified_reason", "")
            unverified = validity == analyze.Validity.VALID and (not pinned or c.get("unverified_reason"))
            c["validity"] = analyze.Validity.UNVERIFIED if unverified else validity
            c["validity_reason"] = reason
            if c["validity"] == "invalid":
                c.update(value=None, ci=[None, None], n=0, note=c["validity_reason"])


PER_BUILD_METRICS = ["qps@recall0.90", "qps@recall0.95", "qps@recall0.99", "build_seconds", "build_peak_rss_mib", "serving_rss_mib", "index_bytes_mib", "cold_start_s"]
LABELS = {k: (label, unit, dec) for k, label, unit, dec in ANN_METRICS}


def per_build_for(path, data):
    """Per-build comparison of one embedded ANN file: for every (comparator, build) one entry per metric, WARM
    against that build's own value of that metric. No entry compares values taken from different builds.
    Returns (entries, level); entries carry both cells, the advantage and the verdict."""
    ref = reference(data)
    if data["kind"] != "ann" or is_server(data) or ref not in data["systems"]:
        return [], 0.95

    def entries_at(level):
        built = analyze.per_build_metrics(rel(path), data, level=level)
        for cells in built.values():
            _mark_validity(data, {m: {"x": c} for m, c in cells.items()})
        mine_by = {m: c for (system, _), cells in built.items() if system == ref for m, c in cells.items()}
        out = []
        for (system, build), cells in built.items():
            if system == ref:
                continue
            for metric in PER_BUILD_METRICS:
                theirs, mine = cells.get(metric), mine_by.get(metric)
                if theirs is None or mine is None or theirs["value"] is None or mine["value"] is None:
                    continue
                adv = advantage(metric, mine, theirs)
                v = "n/v" if metric in analyze.NO_VERDICT else verdict(adv)
                out.append({"system": system, "build": build, "metric": metric, "mine": mine, "theirs": theirs, "adv": adv, "verdict": v})
        return out, built
    first, _ = entries_at(0.95)
    family = sum(1 for e in first if e["verdict"] in ("WIN", "TIE", "LOSS"))
    level = 1 - 0.05 / max(family, 1)
    entries, built = entries_at(level)
    return entries, level


def metrics_for(path, data, siblings=()):
    """Cells at a Bonferroni-adjusted level: the family is the verdict-bearing metrics of this
    results file (those where WARM and at least one comparator have a value). A file read under two timing
    contracts counts both contracts in one family: the two verdict sets are read side by side."""
    kind = data["kind"]
    fn = analyze.ann_metrics if kind == "ann" else analyze.text_metrics
    spec = ANN_METRICS if kind == "ann" else TEXT_METRICS

    def cells_of(view, level):
        metrics = fn(rel(path), view, level=level)
        _mark_validity(view, metrics)
        return metrics

    def family_of(view):
        first = cells_of(view, 0.95)
        ref = reference(view)
        return sum(1 for k, *_ in spec if not analyze.no_verdict(kind, k, view.get("mode", "embedded")) and ref in first.get(k, {})
                   and first[k][ref]["value"] is not None and best_other(k, first[k])[0] is not None
                   and verdict(advantage(k, first[k][ref], best_other(k, first[k])[1])) in ("WIN", "TIE", "LOSS"))
    family = sum(family_of(v) for v in siblings) if siblings else family_of(data)
    level = 1 - 0.05 / max(family, 1)
    return cells_of(data, level), level, family


def section(path, data, siblings=()):
    kind = data["kind"]
    metrics, level, family = metrics_for(path, data, siblings)
    spec = ANN_METRICS if kind == "ann" else TEXT_METRICS
    hw = data["hardware"]
    ds = data["dataset"]
    title = dataset_label(ds) + (f" ({ds['n_train']:,} vectors, {ds['n_test']:,} queries)" if kind == "ann" else f" ({ds['documents']:,} documents, {ds['queries']:,} queries)")
    if is_server(data):
        title += ", server mode"
    if data.get("contract"):
        title += f", {data['contract']} contract"
    per_build, build_level = per_build_for(path, data)
    return {"path": path, "data": data, "metrics": metrics, "spec": spec, "title": title, "hw": hw, "kind": kind,
            "level": level, "family": family, "per_build": per_build, "build_level": build_level}


def timing_boundary_lines(data):
    boundary = data.get("timing_boundary", {})
    present = [s for s in data["systems"] if s in boundary]
    return ["Timing boundary (where each query is timed): " + "; ".join(f"{display(s, data)}: {boundary[s]}" for s in present) + ".", ""] if present else []


def provenance_lines(data):
    """Session and build-selection facts a reader needs before comparing rows: a WARM row rerun in another session, and
    which builds each comparator ran and why."""
    lines = []
    rerun = data.get("warm_rerun")
    if rerun:
        lines += [f"The WARM row is a rerun in a separate session (commit `{rerun.get('commit', '')[:10]}`, {rerun.get('date', '')[:10]}), not interleaved with the "
                  "comparators' rounds; its factors are resampled unpaired. " + rerun.get("note", ""), ""]
    if data["kind"] == "ann":
        selection = (data.get("protocol") or {}).get("build_selection")
        runs = []
        for s_, entry in data["systems"].items():
            if registry.system(s_)["family"] == "warm":
                continue
            sel = (selection or {}).get(s_)
            basis = f"{len(sel['run'])} of {len(sel['defined'])} defined builds, {sel['basis']}" if sel else "selection not recorded"
            runs.append(f"{display(s_, data)}: {', '.join(entry['builds'])} ({basis})")
        if runs:
            lines += ["Comparator builds run: " + "; ".join(runs) + ".", ""]
    return lines


def contract_lines(data):
    name = data.get("contract")
    if not name:
        return []
    reference_row = reference(data)
    others = [display(s, data) for s in data["systems"] if s != reference_row and registry.system(s)["family"] != "warm"]
    return [f"Timing contract: **{registry.CONTRACT_LABEL[name]}**. Every verdict in this section is the `{reference_row}` row ({display(reference_row, data)}) "
            f"against the fastest comparator of this contract, named in the Best other column. Comparators in this contract: {'; '.join(others)}.", ""]


def interval_text(sec):
    data = sec["data"]
    protocol = data.get("protocol") or {}
    rounds, builds = protocol.get("rounds"), protocol.get("build_repeats")
    counts = f"{rounds} process rounds" + (f" and {builds} build runs" if builds is not None else "") + " per system. "
    few = [what for what, n in (("serve rounds", rounds), ("build runs", builds)) if n is not None and n < analyze.MIN_BOOTSTRAP_ROUNDS]
    return (counts + f"Advantage intervals: a {sec['level'] * 100:.2f}% percentile bootstrap of the factor (ratio of medians; Bonferroni family of "
            f"{sec['family']} verdict metrics), rounds resampled in pairs where both rows ran in the same interleaved rounds and independently otherwise; "
            "nDCG@10 and recall@100 by a paired per-query bootstrap at the same level. A WIN or LOSS needs that bootstrap with at least "
            f"{analyze.MIN_BOOTSTRAP_ROUNDS} observations on each side; with fewer the bracket is the min-max range and the row is a measured ratio "
            "with no verdict (n/v). Each bracket names its kind."
            + (f" This file has fewer than {analyze.MIN_BOOTSTRAP_ROUNDS} {' and '.join(few)}: those rows carry no verdict." if few else ""))


def render_section(sec, cells_out):
    data, metrics, spec, hw = sec["data"], sec["metrics"], sec["spec"], sec["hw"]
    path = sec["path"]
    lines = [f"### {sec['title']}", "",
             f"Suite `{data['suite']}`, hardware class `{hw['class']}` ({hw['cpu']}, {hw['logical_cores']} logical cores, {hw['memory_bytes'] / 2**30:.0f} GiB, " +
             f"{hw['os']}{', GitHub-hosted runner' if hw.get('hosted_runner') else ''}), commit `{data['commit'][:10]}`, {data['date'][:10]}. " +
             f"Raw results: [{path.name}](../../{rel(path)}). {interval_text(sec)}", "",
             *contract_lines(data),
             *provenance_lines(data),
             *timing_boundary_lines(data),
             f"| Metric | {'warm-server' if is_server(data) else 'WARM'} | Best other | Advantage (WARM vs best other) | Verdict |", "|---|---|---|---|---|"]
    tally = {"WIN": [], "TIE": [], "LOSS": [], "n/m": [], "n/v": []}
    footnotes = []

    def marker(key, mine, other, name):
        note = analyze.comparison_note(key, mine, other) if other else ""
        if not note:
            return ""
        footnote = f"{display(reference(data), data)} vs {display(name, data)}: {note}."
        if footnote not in footnotes:
            footnotes.append(footnote)
        return "†"

    embedded_ops = lambda key: key in analyze.OPS_METRICS and not is_server(data)  # noqa: E731
    dropped = {}
    for key, label, unit, dec in spec:
        cells = metrics.get(key)
        if not cells:
            continue
        mine = cells.get(reference(data))
        contract = mine if mine is not None and embedded_ops(key) else None
        name, other = best_other(key, cells, contract)
        if contract is not None and other is None and any(s != reference(data) and c["value"] is not None for s, c in cells.items()):
            # Every comparator that measured this row commits by a different contract: no row, no factor.
            dropped[key] = label
            continue
        for system, c in cells.items():
            # The All systems table prints n/c for a durable-operation contract that differs from WARM's.
            incomparable = embedded_ops(key) and mine is not None and c.get("durability_class") != mine.get("durability_class")
            cells_out.append({"section": sec["title"], "file": rel(path), "hardware_class": hw["class"], "metric": key, "system": system,
                              "value": c["value"], "ci": c["ci"] if c["value"] is not None else None, "n": c["n"], "source": c["source"],
                              "validity": c["validity"], "validity_reason": c["validity_reason"], "uncertainty": c["uncertainty"],
                              "interval_kind": c.get("interval_kind"), "text": "n/c" if incomparable else fmt(c["value"], dec)})
        if mine is None:
            continue
        adv = advantage(key, mine, other) if other else None
        mark = marker(key, mine, other, name)
        v = "n/v" if analyze.no_verdict(sec["kind"], key, data.get("mode", "embedded")) or mark else verdict(adv) if other else "n/m"
        tally[v].append((key, label, name, adv))
        if other is not None:
            cells_out.append({"section": sec["title"], "file": rel(path), "hardware_class": hw["class"], "metric": key, "derived": True,
                              "warm": reference(data), "other": name, "factor": None if adv is None or adv["factor"] == float("inf") else adv["factor"],
                              "factor_infinite": bool(adv and adv["factor"] == float("inf")), "ci": adv["ci"] if adv else None, "verdict": v,
                              "interval_kind": (adv or {}).get("interval_kind"), "text": "n/m" if adv is None else fx(adv["factor"])})
        adv_text = "n/m" if adv is None else fx(adv["factor"]) + (adv_interval_text(adv) or " (no interval)")
        if adv and adv.get("paired"):
            p_ = adv["paired"]
            adv_text += f"; paired difference {p_['difference']:+.4f} [{p_['ci'][0]:+.4f}, {p_['ci'][1]:+.4f}]"
        best_text = "n/m" if other is None else f"{display(name, data)} {fmt(other['value'], dec)}{mark}"
        if analyze.no_verdict(sec["kind"], key, data.get("mode", "embedded")) and key in analyze.ANN_PER_BUILD:
            best_text, adv_text = "per build, see below", "per build"
        note = f" ({mine['note']})" if mine["value"] is None and mine["note"] else ""
        lines.append(f"| {label}{' (' + unit + ')' if unit else ''} | {fmt(mine['value'], dec)}{mark}{note} | {best_text} | {adv_text} | **{v}** |")
    lines += ["", "<details><summary>All systems</summary>", ""]
    systems = list(data["systems"])
    head = ["Metric"] + [display(s, data) for s in systems]
    lines += ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    for key, label, unit, dec in spec:
        cells = metrics.get(key)
        if not cells or key in dropped:
            continue
        sign = 1 if analyze.direction(key) == analyze.HIGHER else -1
        mine = cells.get(reference(data))
        comparable = lambda s_, c_: not (embedded_ops(key) and mine is not None and c_.get("durability_class") != mine.get("durability_class"))  # noqa: E731
        valued = {s: c["value"] for s, c in cells.items() if c["value"] is not None and comparable(s, c)}
        if mine and any(analyze.comparison_note(key, mine, cells[s]) for s in valued):
            valued = {}
        top = max(valued, key=lambda s: sign * valued[s]) if valued else None
        row = [label + (f" ({unit})" if unit else "")]
        for s in systems:
            c = cells.get(s)
            if c is None:
                row.append("")
            elif not comparable(s, c):
                row.append("n/c")
            elif c["value"] is None:
                row.append(f"n/m ({c['note']})" if c["note"] else "n/m")
            else:
                text = fmt(c["value"], dec)
                if mine and s != reference(data):
                    text += marker(key, mine, c, s)
                row.append(f"**{text}**" if s == top else text)
        lines.append("| " + " | ".join(row) + " |")
    lines += ["", "n/c: not comparable, the system's durable-operation contract differs from WARM's (see the commit contracts below)." if any(embedded_ops(k) for k, *_ in spec) else "",
              "Bold marks the best value in the row. Operating points for the QPS rows: "
              + "; ".join(f"{display(s, data)}: {metrics['qps@recall0.95'][s]['note']}" for s in systems
                          if "qps@recall0.95" in metrics and s in metrics["qps@recall0.95"] and metrics["qps@recall0.95"][s]["note"]) + "." if sec["kind"] == "ann" else
              "Bold marks the best value in the row.", "", "</details>", ""]
    lines += [f"† {note}" for note in footnotes] + ([""] if footnotes else [])
    if not is_server(data):
        lines += ["Commit contract of one durable replace or delete, per system (rows compare only systems of the same class):", "",
                  "| System | Class | What one operation does |", "|---|---|---|"]
        lines += [f"| {display(s_, data)} | {registry.commit_contract(s_)[0]} | {registry.commit_contract(s_)[1]} |" for s_ in data["systems"]]
        lines += [""]
        for key, label in dropped.items():
            lines += [f"Row not generated: {label}. Every comparator measured in this file commits by a different contract than WARM, so no comparison exists; "
                      f"a comparator with an incremental durable commit would be needed.", ""]
    if sec["kind"] == "text":
        lines += ingest_mode_lines(sec)
    lines += per_build_lines(sec, cells_out)
    skipped = data.get("skipped")
    if skipped:
        lines += ["Systems that did not run in this file: " + "; ".join(f"`{k}`: {v[:160]}" for k, v in skipped.items()) + ".", ""]
    return lines, tally


def ingest_mode_lines(sec):
    """Both WARM text rows side by side: `warm` (lexical ingest) feeds every verdict, `warm-eager` is reported only."""
    metrics, data = sec["metrics"], sec["data"]
    if registry.WARM not in data["systems"] or registry.WARM_EAGER not in data["systems"]:
        return []
    lines = ["WARM ingest modes. Verdicts above use `warm` (lexical-only ingest with the document text stored). `warm-eager` "
             "(chunk hypervectors built at ingest) is reported for reference and never feeds a verdict:", "",
             "| Metric | warm (feeds verdicts) | warm-eager (reference) | eager / warm |", "|---|---|---|---|"]
    for key, label, unit, dec in sec["spec"]:
        a, b = metrics.get(key, {}).get(registry.WARM), metrics.get(key, {}).get(registry.WARM_EAGER)
        if key not in ("qps", "p50_latency", "p99_latency", "build_seconds", "serving_rss_mib", "disk_mib") or not a or not b or a["value"] is None or b["value"] is None:
            continue
        lines.append(f"| {label}{' (' + unit + ')' if unit else ''} | {fmt(a['value'], dec)} | {fmt(b['value'], dec)} | {b['value'] / a['value']:.2f}x |")
    qa, qb = metrics.get("qps", {}).get(registry.WARM), metrics.get("qps", {}).get(registry.WARM_EAGER)
    if qa and qb and qa["value"] and qb["value"] and (qb["ci"][0] > qa["ci"][1] or qb["ci"][1] < qa["ci"][0]):
        lines += ["", "The two QPS intervals do not overlap although both rows run the same lexical search over the same index: the variance between ingest modes is unexplained, "
                  "so neither figure alone is the engine's speed."]
    return lines + [""]


def per_build_lines(sec, cells_out):
    entries = sec.get("per_build") or []
    if not entries:
        return []
    data, path, level = sec["data"], sec["path"], sec["build_level"]
    ref = reference(data)
    builds = sorted({(e["system"], e["build"]) for e in entries})
    tally = {"WIN": 0, "TIE": 0, "LOSS": 0}
    lines = ["Per-build comparison. Each row is one comparator build; every figure in it, and WARM's factor against it, comes from that build alone "
             f"(QPS at a recall target uses only that build's sweep). Each bracket names its kind: a {level * 100:.2f}% bootstrap of the factor, or a min-max range that carries no verdict.", "",
             "| Comparator build | " + " | ".join(f"{LABELS[m][0]} ({LABELS[m][1]})" if LABELS[m][1] else LABELS[m][0] for m in PER_BUILD_METRICS) + " |",
             "|---|" + "---|" * len(PER_BUILD_METRICS)]
    by = {(e["system"], e["build"], e["metric"]): e for e in entries}
    for system, build in builds:
        row = [f"{display(system, data)} {build}"]
        for m in PER_BUILD_METRICS:
            e = by.get((system, build, m))
            if e is None:
                row.append("n/m")
                continue
            dec = LABELS[m][2]
            adv = e["adv"]
            text = f"{fmt(e['theirs']['value'], dec)}; WARM {fmt(e['mine']['value'], dec)}"
            if e["verdict"] in tally:
                tally[e["verdict"]] += 1
            if adv is not None and m not in analyze.NO_VERDICT:
                text += f", {fx(adv['factor'])}" + adv_interval_text(adv) + f" **{e['verdict']}**"
            row.append(text)
            for who, cell_ in ((ref, e["mine"]), (system, e["theirs"])):
                cells_out.append({"section": sec["title"], "file": rel(path), "hardware_class": sec["hw"]["class"], "metric": m, "system": who, "build": build if who == system else None,
                                  "per_build": [system, build], "value": cell_["value"], "ci": cell_["ci"], "n": cell_["n"], "source": cell_["source"],
                                  "validity": cell_["validity"], "validity_reason": cell_["validity_reason"], "uncertainty": cell_["uncertainty"],
                                  "interval_kind": cell_.get("interval_kind"), "text": fmt(cell_["value"], dec)})
            if m in analyze.NO_VERDICT:
                continue
            cells_out.append({"section": sec["title"], "file": rel(path), "hardware_class": sec["hw"]["class"], "metric": m, "derived": True, "per_build": [system, build],
                              "warm": ref, "other": system, "factor": None if adv is None or adv["factor"] == float("inf") else adv["factor"],
                              "factor_infinite": bool(adv and adv["factor"] == float("inf")), "ci": adv["ci"] if adv else None, "verdict": e["verdict"],
                              "interval_kind": (adv or {}).get("interval_kind"), "text": "n/m" if adv is None else fx(adv["factor"])})
        lines.append("| " + " | ".join(row) + " |")
    lines += ["", f"Per-build verdicts: WARM wins {tally['WIN']}, ties {tally['TIE']}, losses {tally['LOSS']}. Index size carries no verdict.", ""]
    return lines


def facts_table(results):
    seen = {}
    for _, data in results:
        for system in data["systems"]:
            seen.setdefault(system, data["facts"].get(system, {}))
    lines = ["| System | Service needed | Encrypted at rest | Basis |", "|---|---|---|---|"]
    for system, facts in seen.items():
        svc, enc = facts.get("service_needed", {}), facts.get("encrypted_at_rest", {})
        basis = "measured: " + enc["source"] if enc.get("basis") == "measured" else "documentation (not exercised)"
        lines.append(f"| {display(system)} | {svc.get('value', 'n/m')} | {enc.get('value', 'n/m')} | {basis} |")
    return lines


def versions_table(results):
    seen = {}
    for _, data in results:
        for system, entry in data["systems"].items():
            text = ", ".join(f"{k} {v}" for k, v in (entry.get("version") or {}).items())
            if text not in seen.setdefault(system, []):
                seen[system].append(text)
    lines = ["| System | Pinned version(s) |", "|---|---|"]
    for system, texts in seen.items():
        lines.append(f"| {display(system)} | {'; '.join(texts)} |")
    return lines


def across_hardware_lines(sections):
    """Text QPS verdict of every (corpus, contract) on every hardware class, plus the groups whose verdict differs between classes.
    Each row is the section's own verdict (same Bonferroni family); rows of different classes are listed side by side, never pooled."""
    rows = {}
    for sec in sections:
        data = sec["data"]
        if sec["kind"] != "text" or is_server(data) or not data.get("contract"):
            continue
        cells = sec["metrics"].get("qps")
        mine = (cells or {}).get(reference(data))
        name, other = best_other("qps", cells) if cells else (None, None)
        if mine is None or mine["value"] is None or other is None:
            continue
        adv = advantage("qps", mine, other)
        rows.setdefault((dataset_label(data["dataset"]), data["contract"]), []).append((sec["hw"]["class"], data, mine, name, other, adv, verdict(adv)))
    rows = {k: sorted(v, key=lambda r: r[0]) for k, v in sorted(rows.items()) if len(v) > 1}
    if not rows:
        return []
    out = ["## Text verdicts across hardware classes", "",
           "The same campaign (suite `standard`, one pinned core, interleaved rounds; counts in each section) on each hardware class. Each row is that class's own single-thread QPS verdict "
           "of the WARM reference row of the contract against the fastest comparator of the same contract; the rows are never pooled or compared as absolute values across classes.", "",
           "| Dataset | Contract | Hardware class | WARM QPS | Fastest comparator | Comparator QPS | Advantage | Verdict |", "|---|---|---|---|---|---|---|---|"]
    differ = []
    for (ds, contract), group in rows.items():
        for hw, data, mine, name, other, adv, v in group:
            ci = adv_interval_text(adv)
            out.append(f"| {ds} | {contract} | `{hw}` | {fmt(mine['value'], 0)} | {display(name, data)} | {fmt(other['value'], 0)} | {fx(adv['factor']) if adv else 'n/m'}{ci} | **{v}** |")
        if len({r[6] for r in group}) > 1:
            differ.append(f"{ds}, {contract} contract: " + "; ".join(f"`{r[0]}` {r[6]}" for r in group))
    out += ["", "Verdicts that differ between hardware classes: " + ("none." if not differ else ""), *[f"- {d}" for d in differ], ""]
    return out


def build():
    # Embedded sections first, then server-mode sections under their own heading: a server row is
    # never compared with an embedded one.
    results = sorted(load_results(), key=lambda pd: is_server(pd[1]))
    sections = [section(p, d, [d2 for p2, d2 in results if p2 == p and d.get("contract") and d2.get("contract")]) for p, d in results]
    cells = []
    body, server_body, summary, losses = [], [], [], []
    for sec in sections:
        lines, tally = render_section(sec, cells)
        if is_server(sec["data"]):
            server_body += lines
        else:
            body += lines
        counts = {k: len(v) for k, v in tally.items()}
        summary.append((sec, counts, tally))
        for key, label, name, adv in tally["LOSS"]:
            losses.append((sec, key, label, name, adv))
        for e in sec["per_build"]:
            if e["verdict"] == "LOSS":
                losses.append((sec, e["metric"], f"{LABELS[e['metric']][0]}, per build", e["system"] + "|" + e["build"], e["adv"]))
    out = ["# WARM comparison", "",
           "Generated by `./benchmarks/competitive/warm-bench table` from the committed files in `benchmarks/results/competitive/`; do not edit by hand. " +
           "Every cell is indexed with its source file and key path in [comparison.cells.json](comparison.cells.json). Method, caveats and how to reproduce: [index.md](index.md).", "",
           "Rows are never compared across hardware classes: each section is one dataset on one hardware class. " +
           f"A verdict is WIN or LOSS only when a 95% (Bonferroni-adjusted) percentile bootstrap interval of the advantage factor itself excludes 1, with at least {analyze.MIN_BOOTSTRAP_ROUNDS} "
           + "process rounds or build runs on each side (quality metrics: a paired per-query bootstrap); deterministic byte counts use a 5% band; a bootstrap interval that "
           + "contains 1 is a TIE. Rows with fewer observations, or with an unverified side, print the measured factor and its min-max range with no verdict (n/v), and the ones where "
           + "WARM is behind are listed under \"Measured behind, no verdict\". " +
           "The advantage factor is always oriented so that more than 1 means WARM is better. Hosted-runner rows carry noise; trust the ratios, not the absolute values.", "",
           "## Scoreboard", "", "| Dataset | Hardware class | Suite | WARM wins | Ties | Losses | Not measured | Informational (no verdict) |", "|---|---|---|---|---|---|---|---|"]
    for sec, counts, _ in summary:
        out.append(f"| {sec['title']} | `{sec['hw']['class']}` | {sec['data']['suite']} | {counts['WIN']} | {counts['TIE']} | {counts['LOSS']} | {counts['n/m']} | {counts['n/v']} |")
    out += [""] + across_hardware_lines(sections)
    out += ["## Where WARM loses", ""]
    if losses:
        out += ["| Dataset | Metric | Best other | WARM disadvantage | Candidate lever (hypothesis unless a file is cited) |", "|---|---|---|---|---|"]
        for sec, key, label, name, adv in losses:
            out.append(f"| {sec['title']} ({sec['hw']['class']}) | {label} | {display(name.split('|')[0], sec['data']) + (' ' + name.split('|')[1] if '|' in name else '')} | {'the comparator has 0' if adv['factor'] == 0 else format(1 / adv['factor'], '.2f') + 'x worse'} | {LEVERS.get(sec['kind'] + ':' + key) or LEVERS.get(key, '')} |")
    else:
        out.append("No metric in the committed results is lost.")
    behind_rows = [(sec, *row) for sec, _, tally in summary for row in behind(sec, tally)]
    behind_rows += [(sec, e["metric"], f"{LABELS[e['metric']][0]}, per build", e["system"] + "|" + e["build"], e["adv"], no_verdict_reason(e["adv"], e["mine"], e["theirs"]))
                    for sec, _, _ in summary for e in sec["per_build"]
                    if e["verdict"] == "n/v" and e["metric"] not in analyze.NO_VERDICT and e["adv"] and e["adv"].get("factor") is not None and e["adv"]["factor"] < 1]
    if behind_rows:
        out += ["", "### Measured behind, no verdict", "",
                "Rows whose measured factor puts WARM behind but which carry no verdict under the rule above (too few observations for a bootstrap, "
                + "or a side whose measurement is not verified). They are losses as measured, not established ones.", "",
                "| Dataset | Metric | Best other | WARM disadvantage (measured) | Interval | Why no verdict |", "|---|---|---|---|---|---|"]
        for sec, key, label, name, adv, reason in behind_rows:
            who = display(name.split('|')[0], sec['data']) + (' ' + name.split('|')[1] if '|' in name else '')
            worse = 'the comparator has 0' if adv['factor'] == 0 else format(1 / adv['factor'], '.2f') + 'x worse'
            out.append(f"| {sec['title']} ({sec['hw']['class']}) | {label} | {who} | {worse} | {adv_interval_text(adv).strip() or 'none'} | {reason} |")
    out += ["", "## Results", ""] + body
    if server_body:
        out += ["## Server mode results", "",
                ("Every system here is a separate server process tree measured from one client over loopback; verdicts compare "
                 "comparators with `warm-server`, never with the embedded rows above. The protocol is recorded in each results file."), ""] + server_body
    from . import jsvector
    js_results = jsvector.load()
    if js_results:
        out += jsvector.lines(js_results, cells) + jsvector.loss_lines(js_results)
    out += ["## Operational facts", "", *facts_table(results), "",
                                           "## Versions", "", *versions_table(results), ""]
    nr = {}
    for _, data in results:
        nr.update(data.get("not_run", {}))
    nr.update(registry.CAMPAIGN_NOT_RUN)  # platform-independent: adapters the published campaign does not run
    out += ["## Not run", "", *[f"- `{k}`: {v}" for k, v in nr.items()], ""]
    block = readme_block(summary)
    return "\n".join(out), cells, block, gaps_from(summary)


CODE_PATHS = {
    "ann": {"default": "src/core/qgraph/vertex.rs (VGraph search/refine), src/bin/research/public-bench.rs (ann-qgraph)",
            "build_seconds": "src/core/qgraph (Graph::build_owned, VGraph::from_graph_with)",
            "cold_start_s": "no persisted VGraph exists; Graph::save/load only cache the structure (src/core/qgraph)",
            "serving_rss_mib": "src/core/qgraph/vertex.rs (index arrays), build-time allocations in Graph::build_owned"},
    "text": {"default": "src/core/documents.rs (prepare, search), src/core/documents/search.rs, src/core/engine/document_api.rs",
             "build_seconds": "src/core/documents.rs prepare + src/core/engine/document_api.rs memorize_documents",
             "cold_start_s": "src/core/storage.rs and src/core/engine/mod.rs (open: arena and document map load)",
             "serving_rss_mib": "src/core/storage.rs (arena), src/core/documents/index.rs, postings.rs",
             "disk_mib": "src/core/storage.rs (arena preallocation), src/core/documents/postings.rs",
             "replace_ms_per_op": "src/core/engine/mod.rs commit (one durable transaction per call), src/core/durable_file.rs",
             "delete_ms_per_op": "src/core/engine/mod.rs commit (one durable transaction per call), src/core/durable_file.rs"},
}


BEHIND = "BEHIND, NO VERDICT"


def no_verdict_reason(adv, mine, other):
    """Why a measured factor carries no verdict, in words."""
    if adv.get("unverified"):
        return "; ".join(dict.fromkeys(c.get("validity_reason") or "unverified measurement" for c in (mine, other) if c.get("validity", "valid") != "valid"))
    if adv.get("interval_kind") == "range":
        return f"{adv['observations'][0]} and {adv['observations'][1]} observations, fewer than the {analyze.MIN_BOOTSTRAP_ROUNDS} a verdict needs"
    return "no interval of the factor"


def behind(sec, tally):
    """(key, label, name, adv, reason) for every verdict-free row whose measured factor puts WARM behind: a stricter verdict
    rule must not make a measured disadvantage disappear from the loss lists."""
    out = []
    for key, label, name, adv in tally["n/v"]:
        if analyze.no_verdict(sec["kind"], key, sec["data"].get("mode", "embedded")) or not adv or adv.get("factor") is None or adv["factor"] >= 1:
            continue
        cells = sec["metrics"][key]
        out.append((key, label, name, adv, no_verdict_reason(adv, cells[reference(sec["data"])], cells[name])))
    return out


def gaps_from(summary):
    """Every metric where WARM loses or ties, and every verdict-free row where the measured factor puts WARM behind, with
    the numbers and the best hypothesis."""
    gaps = []
    for sec, _, tally in summary:
        rows = {"LOSS": [(*r, "") for r in tally["LOSS"]], "TIE": [(*r, "") for r in tally["TIE"]], BEHIND: behind(sec, tally)}
        for status in ("LOSS", "TIE", BEHIND):
            for key, label, name, adv, reason in rows[status]:
                cells = sec["metrics"][key]
                mine, other = cells[reference(sec["data"])], cells[name]
                paths = CODE_PATHS[sec["kind"]]
                gaps.append({
                    "status": status, "dataset": sec["title"], "hardware_class": sec["hw"]["class"], "suite": sec["data"]["suite"],
                    "metric": key, "label": label, "comparator": display(name, sec["data"]),
                    "warm": mine["value"], "comparator_value": other["value"], "unit": mine["unit"],
                    "advantage_factor": adv["factor"] if adv and adv["factor"] != float("inf") else None,
                    "advantage_ci": adv["ci"] if adv else None, "interval_kind": (adv or {}).get("interval_kind"), "no_verdict_reason": reason,
                    "level": sec["level"], "file": rel(sec["path"]),
                    "hypothesis": LEVERS.get(sec["kind"] + ":" + key) or LEVERS.get(key, ""),
                    "code_path": paths.get(key, paths["default"]),
                })
    for sec, _, _ in summary:
        for e in sec["per_build"]:
            adv, unit = e["adv"], e["mine"]["unit"]
            status = e["verdict"]
            if status == "n/v" and e["metric"] not in analyze.NO_VERDICT and adv and adv.get("factor") is not None and adv["factor"] < 1:
                status = BEHIND
            if status not in ("LOSS", "TIE", BEHIND):
                continue
            gaps.append({
                "status": status, "dataset": sec["title"], "hardware_class": sec["hw"]["class"], "suite": sec["data"]["suite"],
                "metric": e["metric"], "label": LABELS[e["metric"]][0] + ", per build", "comparator": f"{display(e['system'], sec['data'])} {e['build']}",
                "warm": e["mine"]["value"], "comparator_value": e["theirs"]["value"], "unit": unit,
                "advantage_factor": adv["factor"] if adv and adv["factor"] != float("inf") else None,
                "advantage_ci": adv["ci"] if adv else None, "interval_kind": (adv or {}).get("interval_kind"),
                "no_verdict_reason": no_verdict_reason(adv, e["mine"], e["theirs"]) if status == BEHIND else "",
                "level": sec["build_level"], "file": rel(sec["path"]),
                "hypothesis": LEVERS.get(e["metric"], ""), "code_path": CODE_PATHS["ann"].get(e["metric"], CODE_PATHS["ann"]["default"]),
            })
    return gaps


def readme_block(summary):
    lines = [START, "", "_Generated from `benchmarks/results/competitive/` by `warm-bench table`; method and caveats in [docs/benchmarks/COMPARISON.md](docs/benchmarks/COMPARISON.md). " +
             f"W = WARM ahead (95% bootstrap interval of the factor excludes parity, at least {analyze.MIN_BOOTSTRAP_ROUNDS} rounds a side), T = tie, L = a comparator is ahead, "
             + "- = no verdict (fewer rounds than the rule needs, unverified measurement or source revision, no interval, or not comparable): the factor is as measured. "
             + "Rows are compared only within one machine._", "",
             "| Dataset | Machine | Metric | WARM | Best other | Advantage | |", "|---|---|---|---|---|---|---|"]
    for sec, counts, tally in summary:
        if sec["data"]["suite"] == "smoke":
            continue
        spec = {k: (label, unit, dec) for k, label, unit, dec in sec["spec"]}
        short = sec["title"].split(" (")[0] + (f" ({sec['data']['contract']})" if sec["data"].get("contract") else "")
        mode = sec["data"].get("mode", "embedded")
        for key in HEADLINE[sec["kind"]]:
            if key in analyze.ANN_PER_BUILD and analyze.no_verdict(sec["kind"], key, mode):
                continue  # compared per comparator build; see the pointer row below
            cells = sec["metrics"].get(key)
            ref = reference(sec["data"])
            if not cells or ref not in cells:
                continue
            label, unit, dec = spec[key]
            mine = cells[ref]
            name, other = best_other(key, cells)
            adv = advantage(key, mine, other) if other else None
            mark = {"WIN": "W", "TIE": "T", "LOSS": "L", "n/m": "-", "n/v": "-"}["n/v" if analyze.no_verdict(sec["kind"], key, mode) else verdict(adv) if other else "n/m"]
            best_text = "n/m" if other is None else f"{display(name, sec['data'])} {fmt(other['value'], dec)}"
            lines.append(f"| {short} | `{sec['hw']['class']}` | {label}{' (' + unit + ')' if unit else ''} | {fmt(mine['value'], dec)} | {best_text} | " +
                         f"{'n/m' if adv is None else fx(adv['factor'])} | {mark} |")
            eager = sec["metrics"].get(key, {}).get(registry.WARM_EAGER) if key == "qps" else None
            if eager and eager["value"] is not None:
                lines.append(f"| {short} | `{sec['hw']['class']}` | {label} ({unit}), WARM with eager ingest (reference, feeds no verdict) | {fmt(eager['value'], dec)} | - | - | - |")
            lexapi = sec["metrics"].get(key, {}).get(registry.WARM_LEXICAL_API) if key == "qps" else None
            if lexapi and lexapi["value"] is not None and ref != registry.WARM_LEXICAL_API:
                lines.append(f"| {short} | `{sec['hw']['class']}` | {label} ({unit}), WARM lexical-only API `search_lexical` (reference, feeds no verdict) | {fmt(lexapi['value'], dec)} | - | - | - |")
        if sec["per_build"]:
            counts = {v: sum(1 for e in sec["per_build"] if e["metric"] in ("build_seconds", "serving_rss_mib") and e["verdict"] == v) for v in ("WIN", "TIE", "LOSS")}
            lines.append(f"| {short} | `{sec['hw']['class']}` | Build time and serving RSS, per comparator build | - | see COMPARISON.md | W {counts['WIN']} / T {counts['TIE']} / L {counts['LOSS']} | |")
    from . import jsvector
    lines += jsvector.readme_rows(jsvector.load())
    lines += ["", END]
    return "\n".join(lines) + "\n"


def update_readme(block, path=README):
    if not path.exists():
        return False
    text = path.read_text()
    if START not in text or END not in text:
        return False
    new = re.sub(re.escape(START) + r".*?" + re.escape(END) + r"\n?", lambda _: block, text, count=1, flags=re.S)
    if new != text:
        path.write_text(new)
    return True


def main(check=False):
    markdown, cells, block, _ = build()
    index = json.dumps(cells, indent=1, allow_nan=False) + "\n"
    targets = {COMPARISON: markdown + "\n", CELLS: index, BLOCK: block}
    if check:
        stale = [rel(p) for p, text in targets.items() if not p.exists() or p.read_text() != text]
        if stale:
            print("stale generated files: " + ", ".join(stale), file=sys.stderr)
            return 1
        return 0
    DOCS.mkdir(parents=True, exist_ok=True)
    for p, text in targets.items():
        p.write_text(text)
    update_readme(block)
    print(f"wrote {rel(COMPARISON)}, {rel(CELLS)}, {rel(BLOCK)}")
    return 0
