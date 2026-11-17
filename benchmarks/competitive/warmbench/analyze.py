# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Pure functions turning one committed results file into per-system metric cells.

A cell is {value, ci, n, source, unit, note}; `source` names the results file and key path the
number was computed from. The table generator and its traceability test both call these
functions, so a table cell can always be recomputed from the committed JSON. `level` is the
confidence level of every interval; the table passes a Bonferroni-adjusted level (the family
is the verdict-bearing metrics of one section).
"""

import statistics
from enum import Enum

from . import registry
from .common import bootstrap_ci, pass_summary, paired_bootstrap

TARGETS = (0.90, 0.95, 0.99)
HIGHER = "higher"
LOWER = "lower"
# Rows that are informational: the two sides are not the same quantity, so no verdict is drawn.
NO_VERDICT = {"install_distributions", "index_bytes_mib"}
# ANN metrics that belong to one comparator build. WARM has one build; a comparator has several, each with its own
# build time, serving memory, index size and QPS. They are compared build by build (per_build_metrics), never as a
# best-of cell that takes each metric from whichever build suits it.
ANN_PER_BUILD = {"build_seconds", "build_peak_rss_mib", "serving_rss_mib", "cold_start_s"}
# Fewer rounds than this and a percentile bootstrap has too few distinct resamples to be a 95 percent interval
# (3 observations give at most 10); the interval reported is then the min-max range of the rounds, and a comparison
# with fewer than this many observations on either side is a measured ratio without a verdict (table.verdict).
MIN_BOOTSTRAP_ROUNDS = 8
OPS_METRICS = ("replace_ms_per_op", "delete_ms_per_op")


def no_verdict(kind, metric, mode="embedded"):
    return metric in NO_VERDICT or (kind == "ann" and mode != "server" and metric in ANN_PER_BUILD)


def interval(values, level):
    values = list(values)
    if 1 < len(values) < MIN_BOOTSTRAP_ROUNDS:
        return [min(values), max(values)]
    return bootstrap_ci(values, level=level)


def interval_kind(n):
    """What a cell's interval over `n` observations is: a percentile bootstrap, the min-max range, or none."""
    return "bootstrap" if n >= MIN_BOOTSTRAP_ROUNDS else "range" if n > 1 else "none"


def sampled(values, level, source, unit, note="", pairing=None):
    """Cell of a metric observed once per process round (or build run). It keeps the per-observation values so the
    advantage factor gets its own bootstrap (table.advantage); `pairing` names the interleaved rounds the values
    belong to: two cells with the same pairing were measured in the same rounds and are resampled together."""
    values = list(values)
    return cell(_med(values), interval(values, level), len(values), source, unit, note) | {
        "rounds": values, "pairing": pairing, "level": level, "interval_kind": interval_kind(len(values))}


class Validity(str, Enum):
    VALID = "valid"
    UNVERIFIED = "unverified"
    INVALID = "invalid"


class Uncertainty(str, Enum):
    SAMPLED = "sampled"
    UNREPLICATED = "unreplicated"
    DETERMINISTIC = "deterministic"


def cell(value, ci=None, n=1, source="", unit="", note="", uncertainty=None):
    uncertainty = Uncertainty(uncertainty or ("sampled" if ci is not None and n > 1 else "unreplicated"))
    return {"value": value, "ci": ci if ci is not None else [value, value], "n": n, "source": source,
            "unit": unit, "note": note, "uncertainty": uncertainty, "validity": Validity.VALID}


def _operation_cell(results, system, *args, **kwargs):
    c = cell(*args, **kwargs)
    if results.get("mode") == "server":
        c["durability_class"] = results.get("protocol", {}).get("durability_class", {}).get(system)
    else:
        c["durability_class"] = registry.commit_contract(system)[0]
    return c


def comparison_note(metric, mine, theirs):
    for c in (mine, theirs):
        if c.get("validity", "valid") != "valid":
            return c.get("validity_reason", "unverified measurement") + "; no verdict"
    if metric not in ("replace_ms_per_op", "delete_ms_per_op"):
        return ""
    if mine["value"] is None or theirs["value"] is None:
        return ""
    if "durability_class" not in mine and "durability_class" not in theirs:
        return ""
    m, t = mine.get("durability_class"), theirs.get("durability_class")
    if m and t and m == t:
        return ""
    return f"durability classes {m or 'unrecorded'} vs {t or 'unrecorded'}; no verdict"


def _med(values):
    return statistics.median(values)


def _half(config, name):
    """The `name` half of a config (recall, repeats) when the worker split the queries, else the whole set."""
    return config.get(name) or config


def split_present(builds):
    return any("select" in c for b in builds.values() for r in b["serve_rounds"] for c in r["configs"])


def pick_config(builds, target):
    """Choose (build_id, config index) by pooled-median QPS among rows whose recall reaches `target`.

    Selection uses the selection half of the queries when the run recorded one (even-indexed queries), pools every
    round so one lucky round cannot pick the row, and never looks at the report half; the report half then carries
    the QPS and the interval. Files written before the split fall back to the whole query set for both.
    Returns (best, closest): `best` is (pooled qps, build id, index, recall) or None; `closest` is the highest
    recall seen.
    """
    best = None
    closest = (-1.0, None)
    for build_id, build in builds.items():
        rounds = build["serve_rounds"]
        if not rounds:
            continue
        for idx in range(len(rounds[0]["configs"])):
            views = [_half(r["configs"][idx], "select") for r in rounds if idx < len(r["configs"])]
            recall = min(v["recall_at_10"] for v in views)
            if recall > closest[0]:
                closest = (recall, (build_id, idx))
            if recall < target:
                continue
            pooled = _med([x["qps"] for v in views for x in v["repeats"]])
            if best is None or pooled > best[0]:
                best = (pooled, build_id, idx, recall)
    return best, closest


def per_build_metrics(path, results, level=0.95):
    """{(system, build_id): {metric: cell}} with every metric of that one build: QPS at each recall target
    (selection and report halves of that build alone), build time and memory, serving memory, index size."""
    out = {}
    for system, entry in results["systems"].items():
        for build_id, build in entry["builds"].items():
            narrowed = {**results, "systems": {system: {**entry, "builds": {build_id: build}}}}
            cells = ann_metrics(path, narrowed, level=level)
            out[(system, build_id)] = {metric: by[system] for metric, by in cells.items() if system in by}
    return out


def _install_cells(put, system, entry, src):
    inst = entry.get("install")
    if not inst:
        return
    if "bytes" in inst:
        # A comparator that ships with the interpreter installs nothing: 0 is not a result WARM can lose to.
        info = bool(inst.get("ships_with_interpreter"))
        note = "ships with the Python interpreter: not an install cost, no verdict drawn against it" if info else ""
        put("install_mib", system, cell(inst["bytes"] / 2**20, None, 1, f"{src}.install", "MiB", note or inst.get("note", ""), uncertainty="deterministic") | {"informational": info})
        put("install_distributions", system, cell(inst.get("distributions", 0), None, 1, f"{src}.install", "count",
                                                  note or "Python distributions (wheels) the install pulls in") | {"informational": info})
        return
    binary = next(v for k, v in inst.items() if not k.startswith("_"))
    # Files written before the binaries were built with the `dist` profile record no symbols field.
    symbols = binary.get("symbols", "included")
    put("install_mib", system, cell(binary["bytes"] / 2**20, None, 1, f"{src}.install", "MiB", f"one statically linked binary, symbols {symbols}"))
    if "_crate_dependencies" not in inst:  # a binary whose dependency tree was not recorded has no count to compare
        return
    crates = inst["_crate_dependencies"]["count"]
    put("install_distributions", system, cell(crates, None, 1, f"{src}.install._crate_dependencies", "count",
                                              "crates compiled into the binary (not separately installed); non-OS shared libraries: "
                                              f"{binary['non_system_dynamic_libraries']}"))


def ann_metrics(path, results, level=0.95):
    """Metric name -> {system -> cell} for an ANN results file."""
    out = {}

    def put(metric, system, c):
        out.setdefault(metric, {})[system] = c

    for system, entry in results["systems"].items():
        builds = entry["builds"]
        chosen = {}
        # A row rerun in another session (`warm_rerun`) shares no rounds with the comparators: resampled unpaired.
        pairing = f"{path}#warm_rerun" if system == registry.WARM and results.get("warm_rerun") else f"{path}#serve_rounds"
        for t in TARGETS:
            best, closest = pick_config(builds, t)
            tag = f"{t:.2f}"
            if best is None:
                put(f"qps@recall{tag}", system, cell(None, source=f"{path}#systems.{system}.builds", note=f"not reached (max recall {closest[0]:.3f})"))
                continue
            _, build_id, idx, recall = best
            rounds = builds[build_id]["serve_rounds"]
            split = "report" in rounds[0]["configs"][idx]
            reported = [_half(r["configs"][idx], "report") for r in rounds]
            src = f"{path}#systems.{system}.builds.{build_id}.serve_rounds[*].configs[{idx}]" + (".report" if split else "")
            held_out = min(v["recall_at_10"] for v in reported)
            if split and held_out < t:
                put(f"qps@recall{tag}", system, cell(None, source=src, note=f"selected on the selection half at recall {recall:.3f}; report-half recall {held_out:.3f} is below the target"))
                continue
            per_round = [_med([x["qps"] for x in v["repeats"]]) for v in reported]
            how = f"selected on the even-indexed queries, reported on the odd-indexed; report recall {held_out:.3f}" if split else f"recall {recall:.3f}"
            put(f"qps@recall{tag}", system, sampled(per_round, level, src, "QPS", f"{build_id} {rounds[0]['configs'][idx]['params']} {how}", pairing))
            if all("p50_us" in x for v in reported for x in v["repeats"]):
                for name in ("p50_us", "p99_us"):
                    vals = [_med([x[name] for x in v["repeats"]]) for v in reported]
                    put(f"{name[:-3]}_latency@recall{tag}", system, sampled(vals, level, src, "us", pairing=pairing))
            chosen[t] = (build_id, idx)
        build_id = (chosen.get(0.95) or chosen.get(0.90) or (next(iter(builds)), 0))[0]
        build = builds[build_id]
        src = f"{path}#systems.{system}.builds.{build_id}"
        shared = build.get("index_of")
        if shared:
            why = f"serves the {shared} row's index file (built there, same bytes); not a separate build"
            for metric, unit in (("build_seconds", "s"), ("build_peak_rss_mib", "MiB"), ("index_bytes_mib", "MiB")):
                put(metric, system, cell(None, None, 0, src, unit, why))
        else:
            if build["build_runs"]:
                runs, runs_key = build["build_runs"], "build_runs"
                ib = runs[0]["index_bytes"]
                ib_note = "directory size of the persisted index (on disk)"
            else:
                runs, runs_key = [r for r in build["serve_rounds"] if r.get("graph_built")], "serve_rounds[graph_built]"
                ib = build["serve_rounds"][0]["index_bytes"]
                ib_note = "in-memory index_bytes reported by the binary (WARM has no persisted ANN index)"
            bt = [r["build_s"] for r in runs]
            put("build_seconds", system, sampled(bt, level, f"{src}.{runs_key}", "s", build_id))
            pk = [r["peak_rss_bytes"] / 2**20 for r in runs]
            put("build_peak_rss_mib", system, sampled(pk, level, f"{src}.{runs_key}", "MiB"))
            put("index_bytes_mib", system, cell(ib / 2**20, None, 1, src, "MiB", ib_note))
        steady = [r["rss_steady"] / 2**20 for r in build["serve_rounds"] if r.get("rss_steady")]
        if steady:
            put("serving_rss_mib", system, sampled(steady, level, src + ".serve_rounds[*].rss_steady", "MiB", "fresh serving process", pairing))
        cold = [r["cold_start_s"] for r in build["serve_rounds"] if r.get("cold_start_s") is not None]
        put("cold_start_s", system, sampled(cold, level, src + ".serve_rounds[*].cold_start_s", "s", pairing=pairing) if cold
            else cell(None, None, 0, src + ".serve_rounds[*].cold_start_s", "s", "not measured"))
        ops = entry.get("ops", {})
        for name in (("replace", "delete") if "ops" in entry else ()):  # a run without the ops phase has no operation cells
            o = ops.get(name)
            if o and o.get("supported"):
                put(f"{name}_ms_per_op", system, _operation_cell(results, system, o["per_op_ms_durable"], None, 1, f"{path}#systems.{system}.ops.{name}", "ms",
                                                      "includes the explicit save the library requires" if o.get("flush_required") else ""))
            else:
                reason = (o or ops).get("reason") or (o or ops).get("error") or "not supported"
                put(f"{name}_ms_per_op", system, cell(None, None, 0, f"{path}#systems.{system}.ops", "ms", reason))
        _install_cells(put, system, entry, f"{path}#systems.{system}")
    return out


def steady_state_note(system, rounds):
    """Why a JVM row's serve timings are not at a verified steady state, or "". Every Lucene round must record that its
    untimed warm-up met the rule in registry.LUCENE_WARMUP (`warmup_steady`); rounds written before that rule stopped
    on a 5 percent agreement after as few as 3 passes and are not steady-state evidence."""
    if not system.startswith("lucene"):
        return ""
    unsteady = [r.get("warmup_passes") for r in rounds if r.get("warmup_steady") is not True]
    if not unsteady:
        return ""
    passes = sorted(p for p in unsteady if p is not None)
    seen = f" (warm-up passes {passes[0]} to {passes[-1]})" if passes else ""
    return (f"JIT warm-up not verified at steady state in {len(unsteady)} of {len(rounds)} rounds{seen}; "
            f"the rule needs at least {registry.LUCENE_WARMUP['min_passes']} passes and {registry.LUCENE_WARMUP['min_seconds']:g} s, then "
            f"{registry.LUCENE_WARMUP['stable_passes']} passes within {registry.LUCENE_WARMUP['band'] * 100:g} percent")


def build_boundary_note(system, build_runs):
    """Why a WARM text build time is not on the common build boundary (data durable and searchable), or "". The WARM
    build records `close_s` once its ingest time holds the durable close; older runs timed the ingest calls alone."""
    if registry.system(system)["family"] != "warm" or all("close_s" in b for b in build_runs):
        return ""
    return "WARM ingest time excludes the durable close (final checkpoint) that every comparator's build time includes"


def _passes(serve_round):
    """Pass summaries of a serve round. Files written before summaries were stored carry the raw
    per-query nanoseconds as `passes_ns`; they are summarised the same way on read."""
    if "passes" in serve_round:
        return serve_round["passes"]
    return [pass_summary(ns) for ns in serve_round["passes_ns"]]


def text_metrics(path, results, level=0.95):
    out = {}

    def put(metric, system, c):
        out.setdefault(metric, {})[system] = c

    for system, entry in results["systems"].items():
        rounds = entry["serve_rounds"]
        src = f"{path}#systems.{system}"
        q = next((r["quality"] for r in rounds if r.get("quality")), None)
        if q:
            qsrc = src + ".serve_rounds[0].quality"
            per = q["per_query_ndcg_at_10"]
            put("ndcg@10", system, cell(q["ndcg_at_10"], bootstrap_ci(per, statistics.fmean, rounds=1000, level=level), q["queries"], qsrc, "nDCG")
                | {"per_query": per, "level": level})
            put("recall@100", system, cell(q["recall_at_100"], bootstrap_ci(q["per_query_recall_at_100"], statistics.fmean, rounds=1000, level=level),
                                           q["queries"], qsrc, "recall") | {"per_query": q["per_query_recall_at_100"], "level": level})
        qps, p50, p99 = [], [], []
        for r in rounds:
            passes = _passes(r)
            qps.append(_med([p["queries"] / (p["total_ns"] / 1e9) for p in passes]))
            p50.append(_med([p["p50_ns"] / 1e3 for p in passes]))
            p99.append(_med([p["p99_ns"] / 1e3 for p in passes]))
        serve = f"{path}#serve_rounds"
        steady = steady_state_note(system, rounds)
        for metric, values in (("qps", qps), ("p50_latency", p50), ("p99_latency", p99)):
            c = sampled(values, level, src + ".serve_rounds[*].passes", "QPS" if metric == "qps" else "us",
                        results.get("timing_boundary", {}).get(system, "") if metric == "qps" else "", serve)
            put(metric, system, c | ({"unverified_reason": steady} if steady else {}))
        bt = [b["ingest_s"] for b in entry["build_runs"]]
        boundary = build_boundary_note(system, entry["build_runs"])
        put("build_seconds", system, sampled(bt, level, src + ".build_runs", "s") | ({"unverified_reason": boundary} if boundary else {}))
        pk = [b["peak_rss_bytes"] / 2**20 for b in entry["build_runs"]]
        put("build_peak_rss_mib", system, sampled(pk, level, src + ".build_runs", "MiB"))
        sv = [r["peak_rss_bytes"] / 2**20 for r in rounds]
        put("serving_rss_mib", system, sampled(sv, level, src + ".serve_rounds[*].peak_rss_bytes", "MiB", "peak RSS of the serving process", serve))
        put("disk_mib", system, cell(entry["build_runs"][0]["disk_bytes"] / 2**20, None, 1, src + ".build_runs[0].disk_bytes", "MiB", "allocated bytes (du)", uncertainty="deterministic"))
        cs = [r["cold_start_s"] for r in rounds]
        put("cold_start_s", system, sampled(cs, level, src + ".serve_rounds[*].cold_start_s", "s", pairing=serve))
        ops = entry.get("ops", {})
        for name in (("replace", "delete") if "ops" in entry else ()):  # a run without the ops phase has no operation cells
            o = ops.get(name)
            if o and o.get("supported"):
                put(f"{name}_ms_per_op", system, _operation_cell(results, system, o["per_op_ms_durable"], None, 1, f"{src}.ops.{name}", "ms"))
            else:
                put(f"{name}_ms_per_op", system, cell(None, None, 0, f"{src}.ops", "ms", (o or ops).get("reason") or (o or ops).get("error") or "not supported"))
        _install_cells(put, system, entry, src)
    return out


# Quality metrics are scored on the same queries for every system, so two systems are compared by a paired per-query
# bootstrap (common.paired_bootstrap), not by the intervals of their separate means.
PAIRED = ("ndcg@10", "recall@100")


def paired_advantage(metric, mine, theirs):
    """Advantage of `mine` over `theirs` on a paired metric: the ratio of the means with the interval of the paired
    resampling, and the paired difference. Both metrics are higher-is-better."""
    if len(mine["per_query"]) != len(theirs["per_query"]):
        raise ValueError(f"{metric}: {len(mine['per_query'])} and {len(theirs['per_query'])} per-query scores are not the same queries")
    r = paired_bootstrap(mine["per_query"], theirs["per_query"], level=mine["level"])
    return {"factor": r["ratio"], "ci": r["ratio_ci"], "paired": {"difference": r["difference"], "ci": r["difference_ci"], "queries": r["queries"], "level": mine["level"]}}


DIRECTIONS = {"qps": HIGHER, "ndcg@10": HIGHER, "recall@100": HIGHER, **{f"qps@recall{t:.2f}": HIGHER for t in TARGETS}}


def direction(metric):
    return DIRECTIONS.get(metric, LOWER)
