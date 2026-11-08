"""audit_table.py summary.jsonl: markdown miss-audit table from replay.py --audit --heldout output."""
import json, sys

with open(sys.argv[1]) as fh:
    rep = json.loads(fh.read().strip().splitlines()[-1])
print("| step | live | queries | set | misses | recall (tie-aware) | probe miss | scan loss | near tie | duplicate | harness-credited ties | queries with a miss | top 1% queries | top 10% queries | top 1% lists | top 10% lists |")
print("|" + "---|" * 16)
for a in sorted(rep["audits"], key=lambda x: (x["step"], x["set"])):
    c = a["cats"]
    print(f"| {a['step']} | {a.get('live', '')} | {a['queries']} | {a['set']} | {a['misses']} | {a['recall_eff']:.4f} | "
          f"{c.get('probe_miss', 0)} | {c.get('scan_loss', 0)} | {c.get('near_tie', 0)} | {c.get('duplicate', 0)} | "
          f"{a['tie_credited']} | {a['queries_with_miss']} | {a['share_top1pct_queries']:.1%} | {a['share_top10pct_queries']:.1%} | "
          f"{a['share_top1pct_lists']:.1%} | {a['share_top10pct_lists']:.1%} |")
