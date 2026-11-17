"""Miss audit helpers for replay.py / audit_prefix.py: classify every top-10 miss of a search."""
import numpy as np

TIE = 1e-6      # the harness rule: GT entries within 1e-6 of the 10th distance count as true
NEAR = 1e-3     # relative near-tie band reported separately
DUP = 1e-2      # near-duplicate: squared distance to the missed neighbour below 1% of d10


def sqd(a, b):
    d = a.astype(np.float64) - b.astype(np.float64)
    return (d * d).sum(-1)


def audit(idx, Q, res, gt_ids, base, nprobe, k=10):
    """res: (nq, k) returned ids; gt_ids: (nq, >=k) exact ids; base: memmap of the source rows.
    Returns (summary dict, per-miss records)."""
    nq = len(res)
    C = idx.centres()
    miss_q, recs = [], []
    credited = total_m = 0
    per_query = np.zeros(nq, dtype=np.int64)
    for q in range(nq):
        g = gt_ids[q, :k]
        r = res[q]
        if r.min() < 0 or len(r) != k:
            pass
        gx = base[np.asarray(g, dtype=np.int64)]
        d_g = sqd(gx, Q[q])
        d10 = d_g.max()
        rset = set(r.tolist())
        rx = base[np.asarray(np.maximum(r, 0), dtype=np.int64)]
        d_r = sqd(rx, Q[q])
        in_gt = np.array([x in set(g.tolist()) for x in r.tolist()])
        tie_extra = (~in_gt) & (d_r <= d10 * (1 + TIE) + 1e-12)
        hits = int(in_gt.sum() + tie_extra.sum())
        missed = [i for i in range(k) if int(g[i]) not in rset]
        m = max(0, k - hits)
        credited += max(0, len(missed) - m)
        total_m += m
        per_query[q] = m
        if m == 0:
            continue
        # effective misses: the m missed GT neighbours farthest from the query
        missed.sort(key=lambda i: -d_g[i])
        subs = [j for j in range(k) if not in_gt[j]]
        lq = None
        for i in missed[:m]:
            gid = int(g[i])
            l1, l2 = idx.lists_of(np.array([gid], dtype=np.int64))[0]
            if lq is None:
                lq = np.argsort(((C - Q[q]) ** 2).sum(1))
                rank_of = np.empty(len(C), dtype=np.int64)
                rank_of[lq] = np.arange(len(C))
            ranks = [int(rank_of[x]) for x in (l1, l2) if x != np.uint32(0xFFFFFFFF)]
            probed = bool(ranks) and min(ranks) < nprobe
            dd = min((sqd(gx[i], rx[j]) for j in subs), default=np.inf)
            dup = bool(dd <= DUP * d10)
            exact_dup = bool(dd == 0.0)
            near = bool(abs(d_g[i] - d10) <= NEAR * d10 and any(abs(d_r[j] - d_g[i]) <= NEAR * d10 for j in subs))
            cat = "duplicate" if dup else ("near_tie" if near else ("scan_loss" if probed else "probe_miss"))
            recs.append({"q": q, "id": gid, "gt_rank": i, "cat": cat, "exact_dup": exact_dup,
                         "list1": int(l1), "list2": int(l2), "rank_l1": ranks[0] if ranks else -1,
                         "rank_best": min(ranks) if ranks else -1, "d_over_d10": float(d_g[i] / d10)})
    cats = {}
    for x in recs:
        cats[x["cat"]] = cats.get(x["cat"], 0) + 1
    srt = np.sort(per_query)[::-1]
    tot = max(1, int(srt.sum()))
    lists_hit = {}
    for x in recs:
        lists_hit[x["list1"]] = lists_hit.get(x["list1"], 0) + 1
    ls = sorted(lists_hit.values(), reverse=True)
    summ = {"queries": nq, "misses": int(total_m), "recall_eff": 1 - total_m / (nq * k),
            "tie_credited": int(credited), "cats": cats,
            "queries_with_miss": int((per_query > 0).sum()),
            "share_top1pct_queries": float(srt[: max(1, nq // 100)].sum() / tot),
            "share_top10pct_queries": float(srt[: max(1, nq // 10)].sum() / tot),
            "max_miss_one_query": int(srt[0]), "exact_dups": int(sum(x["exact_dup"] for x in recs)),
            "distinct_primary_lists": len(ls),
            "share_top1pct_lists": float(sum(ls[: max(1, len(C) // 100)]) / max(1, sum(ls))),
            "share_top10pct_lists": float(sum(ls[: max(1, len(C) // 10)]) / max(1, sum(ls))),
            "top_lists": sorted(lists_hit.items(), key=lambda kv: -kv[1])[:10]}
    return summ, recs
