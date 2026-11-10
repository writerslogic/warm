# H-B-1 instrument audit

Status: source audit only; no claim that the public ground truth has passed an independent numeric audit. Accuracy-ranked track mechanism measurements must retain both the official score and an audited score once the numeric audit exists. The synthetic H-B-2 counter independently recomputes its oracle and does not score an official row.

Big ANN source was fetched at `ec58203170e3636ba7da82154ce6322de9cb8b33`. Source snapshots are in `sources/bigann/` and hashes in `research.json`.

- `benchmark/plotting/metrics.py:15-46` accepts any returned ID within the stored top-100 prefix whose recorded distance is less than `1e-6` from the stored kth distance. This is a distance-tolerant ID set, not a strict exact top-k ranking. Ties outside the stored top 100 cannot be represented. Duplicate result IDs contribute only once.
- `benchmark/dataset_io.py:133-140` reads stored distances as float32 and IDs as int32. `benchmark/streaming/compute_gt.py:43-88` invokes an external GT executable over the live tag-to-row map; the generated artifact alone does not prove high-precision exactness or a particular floating evaluation order. Shell command failure is not checked before the intermediate data file is removed.
- `benchmark/streaming/compute_gt.py:116` chooses `ds.qs_fn` for both branches of `--private_query`. That flag cannot establish a fresh holdout. Fresh queries need explicit source identity and independently computed truth.
- `benchmark/streaming/load_runbook.py:7-10` stops at the first missing integer step; `max_pts` is trusted metadata. Audit must compare all integer keys, operation ranges, live set membership, replacements and actual maximum live rows rather than trust successful parsing.
- `src/core/qgraph/partition.rs:1209-1255` uses float32 norm-minus-dot ranking before training. `partition.rs:642-651` radius is mean plus two standard deviations, not a containing radius. Neither is a deterministic high-precision correctness certificate.
- Local synthetic `l10-isolation/scripts/gen.py:15-26` stores top-k from float32 matrix products and a float64 norm cast back to float32; H-B-2 deliberately ignores its `gt.npy` and computes direct float64 squared differences. These arrays have previously been inspected and remain development data.

Required numeric audit before any streaming confirmation: freeze exact input hashes and runbook; independently replay tags and replacements; compute direct high-precision distances for all scored fresh queries at every checkpoint; retain official float32/tolerance score and audited exact-distance/tie score, including disagreements with source evidence. Check whether rounding changes membership or only order, and distinguish exact real ties from rounded ties. No correction has yet been proved, and no official label is overridden by this report.

Source discrepancy requiring an organiser ruling before comparison freeze: `neurips23/ongoing_leaderboard/leaderboard.md:10` states leading streaming recall 0.99786, but its linked current CSV has maximum ScaNN 0.9923721875000002. Do not lower the recorded leading reference or infer that the stronger entry is ineligible. The task target remains 1.0 regardless.

OOD source discrepancy: `Azure_D8lds_v5_table.md:15` names Pinecone at 38087.669026 QPS, but `ood/res_public_queries_AzureD8lds_v5.csv` contains hanns at 46033.867231881486 QPS and 0.900311 recall. Record both until organiser eligibility and build accounting are verified; the hanns row reports an implausibly short build time, so the CSV maximum is a published candidate, not independently established eligibility.

VectorDBBench `leaderboard_v2_streaming.json` omits recall. Its throughput maximum alone cannot define an equal-or-better-recall comparison; no mission target achievement may be inferred from those figures. Filter results span different machine/service boundaries; matching their resource labels is necessary before freezing.
