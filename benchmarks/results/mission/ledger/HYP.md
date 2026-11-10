## H-A-1 — baseline and instrument inventory (open)
Limitation: no mission matrix exists; previous evidence uses heterogeneous boundaries.
Mechanism: verify organiser rules and existing result tables before defining comparable rows.
Cost model: one primary-source pass avoids invalid comparisons and wasted benchmark runs.
Risks: stale competitor claims; do not infer missing figures or eligibility.
Experiment: rung 1, bounded source/implementation review in lane A.
Accept: exact version, metrics, constraints and source URLs; reject unsupported claims.
Deliverable: lane inventory, next frontier card with prior art and decisive local test.
## H-B-1 — baseline and instrument inventory (open)
Limitation: no mission matrix exists; previous evidence uses heterogeneous boundaries.
Mechanism: verify organiser rules and existing result tables before defining comparable rows.
Cost model: one primary-source pass avoids invalid comparisons and wasted benchmark runs.
Risks: stale competitor claims; do not infer missing figures or eligibility.
Experiment: rung 1, bounded source/implementation review in lane B.
Accept: exact version, metrics, constraints and source URLs; reject unsupported claims.
Deliverable: lane inventory, next frontier card with prior art and decisive local test.
## H-C-1 — baseline and instrument inventory (open)
Limitation: no mission matrix exists; previous evidence uses heterogeneous boundaries.
Mechanism: verify organiser rules and existing result tables before defining comparable rows.
Cost model: one primary-source pass avoids invalid comparisons and wasted benchmark runs.
Risks: stale competitor claims; do not infer missing figures or eligibility.
Experiment: rung 1, bounded source/implementation review in lane C.
Accept: exact version, metrics, constraints and source URLs; reject unsupported claims.
Deliverable: lane inventory, next frontier card with prior art and decisive local test.

## H-B-2 — certified scalar interval rescue (rejected by E4; seed B5)
Limitation: E2a loses 0.0034288125 recall; Azure repeats take 2865.57/2871.94 s versus 3600 s limit; source partition.rs:1261 probes only nprobe lists and has no exact certificate.
Mechanism: scan 4-bit scalar interval lower bounds outside ordinary approximate retrieval, then exact-rescore every uncertified point; breaks the habit that approximate list selection is final; full coverage in real arithmetic closes the entire residual gap, conditional on affordable work.
Prior art: VA-file (Weber/Schek/Blott, VLDB 1998 https://www.vldb.org/conf/1998/p194.pdf); RaBitQ https://arxiv.org/abs/2405.12497 has probabilistic rather than a deterministic guarantee. No novelty claim; proposed contribution would be bounded rescue composed with existing list-major query tiling.
Primitive model: ordinary PQ50 with dual spill and 650/5000 probing touches about 6.5 packed code bytes per base row/query before overlap/skew; <25% extra work needs <=1.5 rescue code bytes plus <=0.0003*400=0.12 exact-vector bytes per base row/query. These optimistic bytes omit compute, IDs, cache lines and updates; acceptance only advances to measurement.
Prediction: exact rescue eliminates every true miss if sound, but the unfamiliar global scalar box mechanism probably fails the work gate on the harder local distribution; test it rather than silently keeping only attractive mechanisms.
Experiment: certificate_counter.py on all 1M synthetic base rows and the first 16 existing development queries, once per data/data_hard; variance order uses only base vectors; direct float64 L2 independently recomputes kth thresholds; aggregate counters only.
Accept: both datasets mean packed-code bytes <=1.5 and exact-refinement fraction <=0.0003, zero false prunes and zero bound violations; reject this global scalar bound immediately otherwise, preserving B5 with a newly carded different bound family.
Risks: oracle kth thresholds are optimistic; synthetic data is not track evidence; floating margins are checked numerically, not formally proved; dynamic range and deletion require conservative bounds; no candidate may reuse official labels or query IDs.
Transfer: only after passing, test full live snapshots with an independent exact oracle, then dynamic insert/delete/replace patterns and previously untouched queries; finite floats, ties, nonfinite rejection, full resource accounting required before integration.
Validation command: uv run --no-project --python /Volumes/A/.hms-lb/bigann/venv/bin/python benchmarks/results/mission/B/certificate_counter.py --base /Volumes/A/.hms-lb/bigann/l10-isolation/data/base.npy --queries /Volumes/A/.hms-lb/bigann/l10-isolation/data/queries.npy --query-count 16 --output benchmarks/results/mission/B/scalar-bound-data.json (run from /Volumes/A/.hms-wt-mission-b; replace data with data_hard only if first gate passes).
Instrument prerequisite: instrument-audit.md records source findings; script does not trust stored gt.npy, whose generator accumulates mixed float32/float64 distances. Track truth remains unaudited until source vectors and stored GT are compared.

## H-A-2 — one-round completion oracle (rejected E5; seeds A1+A2)
Limitation: serving VGraph expands one candidate at a time (vertex.rs:2737); prior paired/prefetch work only gave small changes, and serial fetch rounds remain uncounted.
Mechanism: separate an unpruned completion list from navigation edges, issue three list fetches together, then contiguous candidate-code scan/rerank; break the convention that navigation and enumeration share one pruned graph.
Prior art: EFANNA https://arxiv.org/abs/1609.07228 uses seeded graph expansion; RoarGraph https://www.vldb.org/pvldb/vol17/p2735-chen.pdf projects query neighborhoods; WARM projection.rs already implements query projection with pruning. No novelty claim.
Primitive model: hypothetical 125 dependent expansions ×60ns +2000 code evaluations ×2ns +1000ns rerank =12.5us; 3 dependent completion rounds ×60ns +384 candidate evaluations ×2ns +3000ns seed stage +1000ns rerank =4.948us, 2.53x projected, conditional and unmeasured; closes >=half a 2x throughput gap if assumptions hold.
Risks: oracle seeds unavailable in deployment; OOD neighbors may not neighbor each other; K128 IDs cost512B/point excluding codes, metadata and original graph; copied codes increase memory; deterministic exact ranking still needed.
Smallest experiment: completion_coverage.py, exposed first32 Cohere queries against first100000 stored vectors; float64 brute-force top100 and exact KNN of oracle top10 seeds; aggregate complete m={1,3,5,10},K={32,64,128,256},k={10,100} grid, one run only.
Prediction: strict top100 coverage>=.99 at m3/K128; this is a conjecture, not evidence.
Acceptance: m3/K128 coverage>=.99 is a necessary-condition pass; require independent OOD transfer and real seed recall before engine integration. Rejection: even m10/K256 coverage<.99 kills base-neighbor-only one-round completion for this workload/size. Intermediate outcome composes with a separately carded query-neighborhood mechanism.
Controls: no engine changes; no answers/queries enter a serving index; all inspected queries are development; timing is not measured; scalar float64 normalization and total (score,row-ID) ordering define oracle; subset says nothing about full-scale leaderboard rank.
Execution: use existing /Volumes/A/.hms-lb/vdbbench/venv Python via uv, --data /Volumes/A/.hms-lb/vdbbench/dataset/cohere/cohere_medium_1m, --rows100000 --queries32, output unique durable JSON, --ledger /Volumes/A/.hms-lb/mission/LOG.tsv, --experiment E<number>; root assigns ID and records look before running.
Next if rejected: query-conditioned completion as a separate card, or existing navigation diagnostics to quantify no-discovery expansions (A3); do not tune K beyond this preregistration.

## H-B-3 — subspace covering-tree certification (rejected E9; seed B5, replaces rejected H-B-2 bound family)
Limitation: E4 rejected global scalar interval scanning at 18.2080158125 ideal code bytes/row/query versus 1.5 cap; retirement applies to that global scan, not exactness. Existing List::radius is mean+2sd, not safe.
Mechanism: a hierarchy of partitions with a true maximum residual norm in each disjoint coordinate subspace; sum max(||q_s-c_s||-R_s,0)^2 over subspaces to bound every row in a partition; prune whole subtrees before touching row codes; refine every remaining row exactly. This breaks per-point verification and approximate nprobe truncation.
Prior art: Omohundro balltrees https://steveomohundro.com/wp-content/uploads/2009/03/omohundro89_five_balltree_construction_algorithms.pdf and multi-step VA-file bound filtering (Weber/Schek/Blott VLDB1998); no novelty claim. Use max(full-ball bound, summed subspace-cover bound): neither dominates universally because subspace maxima can occur on different rows.
Cost ceiling: with 5000 partitions and 25 subspaces, radii occupy 0.5MB; 5000*100 centre-coordinate operations/query are comparable in count to existing centroid routing, then <=1000 exact rows/query would match present reorder count while eliminating all misses; hierarchy may reduce centre visits. These are conditional primitive counts, not timing evidence.
Prediction: if conservative covers leave <=1000 rows/query and <=5000 centre visits, exact search closes 100% of remaining gap with headroom from eliminated PQ scans; if most partitions survive even using oracle kth distances, this family fails before native implementation.
Smallest experiment: first1M development base rows/all available, first16 existing queries; 8192 evenly spaced data-only training rows,64 coarse centers/3 Lloyd iterations,up to78 child centers/1 iteration,4096-row assignment blocks,4-coordinate subspaces; exact float64 oracle; aggregate-only output; no synthetic generator labels. Cap600s and1M rows/128D (mapped raw<=512MB; working arrays<128MB).
Accept: both development distributions mean surviving rows<=1000,sample p99<=5000,<=5000 tested centres,zero enclosure/oracle pruning violations; reject this cover configuration on any threshold failure and do not run the harder distribution. Timeout/failure writes a receipt; no automatic retry or fallback tuning.
Risks: outliers make max radii loose; deletions may conservatively retain old radii, inserts must expand before publication, retrain must atomically replace summaries; floating errors need outward bounds; high-dimensional IP suites need a separate sound bound, not silent L2 conversion.
Transfer: actual MSTuring live snapshots and fresh exact-GT queries before timing; final max_live10292043*100D*4B=4116817200 raw bytes, while retaining all30M source rows costs12GB and violates8GiB. Live reclamation, all remaining structures, maintenance, harness and total<=3600s are unverified; no native resource-feasibility claim.
Command: OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 VECLIB_MAXIMUM_THREADS=1 uv run --no-project --python /Volumes/A/.hms-lb/bigann/venv/bin/python benchmarks/results/mission/B/cover_counter.py --base /Volumes/A/.hms-lb/bigann/l10-isolation/data/base.npy --queries /Volumes/A/.hms-lb/bigann/l10-isolation/data/queries.npy --output benchmarks/results/mission/B/cover-bound-data.json (cwd /Volumes/A/.hms-wt-mission-b; root records LOG; no specialist execution).

## H-A-3 — exact neighborhood domination (open; seed A3)
Limitation: VGraph fetches and visits each popped vertex's adjacency even if all its neighbors were already seen (vertex.rs:2737); the removable fraction is unknown, and rerank costs may provide a competing ceiling.
Mechanism: one precomputed bit on discovery edge p→v certifies N(v)⊆N(p)∪{p}; after p has scanned its whole adjacency, expanding v discovers nothing, so plain traversal can mark v expanded without fetching N(v). This is a sufficient exact certificate, not a learned skip threshold.
Prior art: Speed-ANN https://arxiv.org/abs/2201.13007 and AverSearch https://arxiv.org/abs/2504.20461 remove redundant parallel expansion; novelty of edge-subset certificate unestablished. WARM already has verified NavigationDiagnostic and information-navigation unseen-count policy; reuse their exports, not another tracer.
Primitive model: if certified empty scans consume half total time, removing them gives2x; optimistic equal-expansion ceiling is1/(1-f). Half closure of a2x target from competitor parity needs>=1.5x, requiring f>=1/3 even with zero overhead and100% expansion cost. A slower baseline needs more; no matched throughput prediction is established.
Prediction: zero-discovery expansion fraction>=0.50 and static discovery-parent certificate covers>=1/3 expansions; this conjecture is intentionally threatened by graph diversity pruning.
Smallest experiment: parse existing hash-bound full 10K/100K GloVe navigation traces once; compute zero-offer expansions, exact edge-subset certificates, adjacency reads saved and first-final-answer discovery positions for both frozen ef128/256; all four cells retained.
Secondary ceiling: summarize existing Yi/Llama phase profiles independently; free all rerank/sort time gives1/(1-rerank_sort/total). Never multiply that ceiling by GloVe expansion savings because datasets/configurations differ.
Acceptance: necessary frontier bound only if all four cells have zero-discovery and certified expansion fractions>=1/3; require matched phase costs and independent dataset before implementation. Reject static-domination main line if any cell fails, and reject zero-discovery main line if oracle empty fraction<1/3. No threshold sweep.
Correctness risks: guard only plain traversal without fused block evidence/reservoir side effects; all p adjacency visits must complete before v; updates invalidate certificates atomically; visited mark semantics must be retained. No source edit proposed before ceiling passes.
Evidence: docs/research/milestones-20261102/navigation-{10k,100k}/finished.json bind raw hashes and2000 queries each; production identity verified there. These exposed archives are development, not holdout. Run navigation_headroom.py with --repo integrated --output uniqueJSON --ledger mission/LOG.tsv --experiment E<number>.
Next on rejection: card dependency-depth/pipelining separately using the same trace parent chains; no rescue by relaxing exact subset or extrapolating per-query tail phases into whole-query speed.
