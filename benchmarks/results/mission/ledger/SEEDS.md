SEEDS: CANDIDATE MECHANISMS FOR THE WARM MISSION

Status of everything below: unverified hypothesis.
- Written without access to the WARM source. Some of this may already exist there.
- Novelty is not established. "Prior art" lists only what the author knew, not the result of a search.
- Gain estimates come from rough cost models, not measurements.
- Each entry must be carded, checked for prior art and kept or killed under the mission's rules.

Card fields: From (field borrowed from). Breaks (assumption attacked). Mechanism. Why large. Prior art known. First test. Kill.


Z. AUDIT THE INSTRUMENT BEFORE ANYTHING ELSE

Z1. Instrument audit
From: metrology.
Breaks: the assumption that the official key and judge measure correctness.
Figures to re-verify from primary sources:
- LoCoMo: an independent audit reports 99 of 1,540 gold answers wrong (6.4%), a ceiling near 93.57%, and a judge that accepted up to 63% of deliberately wrong answers. Vendor claims above that ceiling exist. https://github.com/dial481/locomo-audit
- LongMemEval: published highs of 96.4% (Exabase M-1, Gemini 3 Flash), 94.87% (Mastra OM, gpt-5-mini), 94.8% (Mem0). One audit measured judge-versus-human agreement at 97.38%, so the leaders' remaining error is about the size of judge noise. https://arxiv.org/pdf/2607.22690
- Big ANN streaming: the ongoing leaderboard's leading recall is 0.99786 (March 2024 update). https://github.com/harsha-simhadri/big-ann-benchmarks/blob/main/neurips23/ongoing_leaderboard/leaderboard.md
Recall has exact ground truth, so the streaming row has no instrument problem: the remaining 0.00214 is real error and the target is 1.
Action: for the QA rows, build the audited key and judging procedure under the mission's instrument rule and report official and audited scores together. The harder suites now in the inventory (LongMemEval_M, LongMemEval-V2, LoCoMo-Plus, BEAM) are where the leaders sit furthest from a perfect score.

Z2. Deployment-boundary accounting for VectorDBBench
From: operating systems, queueing theory.
Breaks: the assumption that the index is the bottleneck.
Mechanism: count nanoseconds per query spent outside the index: Python list to f32 conversion, FFI crossing, result marshalling, allocator, process model, scheduler. Remove what is not law: buffer-protocol zero-copy input, preallocated outputs, one crossing per query.
Guard: as far as the author knows, the concurrent phase reuses a fixed query set. Verify. Any mechanism that adapts to observed queries must be scored on unseen queries, and result caching is prohibited.
First test: rung 2 timer split, inside-index versus outside-index, at the concurrency that sets the published figure.
Kill: outside-index share below 10%.


A. STATIC SEARCH

A1. Bounded-round search
From: computer architecture (dependent versus independent cache misses), genomics (seed-and-extend alignment).
Breaks: the field's cost model. It counts distance computations and treats a query as one long pointer-chasing traversal. With block-quantised codes an estimate costs about a nanosecond while a dependent DRAM miss costs tens, so the number of dependent fetch rounds sets latency.
Mechanism: (1) seeds from a few independent probes into a small partition or multi-index hash over 1-bit codes, issued together so their misses overlap; (2) fetch the seeds' completion lists (A2) together and scan them in SIMD; (3) optionally one more round from the best results; (4) rerank. Three or four dependent rounds instead of roughly ef.
Why large: an order of magnitude fewer dependent rounds if coverage holds.
Prior art known: LSH-APG and IVF-seeded graphs (entry points only); KGraph and EFANNA (kNN-graph search, set aside under the distance-count cost model); SymphonyQG and NGT-QG (block-quantised adjacency); multi-index hashing.
First test: rung 2, offline, no engine code. From ground truth compute coverage: the fraction of the true top-k inside the union of exact K-NN lists of the true top-m, for m in {1,3,5,10}, K in {32,64,128,256}, k in {10,100}, on one in-distribution and one out-of-distribution dataset. Then seed quality: probability that a cheap seeding stage returns at least m true neighbours.
Kill: coverage below the row's recall target at every K the memory limit allows.

A2. Co-retrieval completion lists
From: neuroscience (Hebbian association), recommender systems (co-occurrence).
Breaks: the assumption that one edge set must serve both navigation and completion. Diversity pruning keeps long edges for navigation and deletes exactly the redundant local edges that enumeration needs.
Mechanism: a second, unpruned list per node holding the points most often returned together with it: exact kNN for in-distribution data, co-occurrence in training-query result sets for out-of-distribution data. Stored as contiguous ids plus codes. Used only in the final phase, alone or under A1.
Prior art known: RoarGraph (query-projected graph), tau-MNG (final-phase monotonicity), NGT ANNG.
First test: in the current search, count expansions spent after the first true neighbour is found, and how many of the remaining true neighbours sit in that node's exact K-NN list.
Kill: fewer than half of late expansions replaceable by one list scan.

A3. Expansion value-of-information pruning
From: decision theory, experimental design.
Breaks: the assumption that a candidate's own distance decides whether to expand it. What an expansion buys is unseen nodes.
Mechanism: one byte per edge v to u holding the number of u's neighbours not already in v's list. In the final phase skip expansions whose novelty is zero or below a threshold unless the candidate ranks in the top few.
Why large: at high recall most expansions confirm rather than discover, and each skipped one saves a dependent miss.
Prior art known: Speed-ANN and AverSearch measure redundant expansion; no per-edge novelty count known to the author.
First test: rung 2 counter. Fraction of expansions that add no unvisited node, and fraction that change the final top-k.
Kill: zero-novelty expansions below 20% at the target recall.

A4. Equimarginal recall budget
From: economics (equimarginal principle), optimal stopping.
Breaks: the assumption that every query gets the same search effort, or must individually reach the recall target. Benchmarks score average recall.
Mechanism: one global price per unit of recall. Each query stops when its predicted marginal recall per unit cost falls below that price. Easy queries finish early, hopeless ones are cut, the average constraint is met at minimum total cost.
Prior art known: LAET, DARTH and related learned early termination. The global-price formulation and its oracle bound may be less explored.
First test: rung 2, offline. Log each query's recall-versus-work curve once. Compute the oracle allocation's total cost at the target average recall against the best fixed setting. That ratio is the ceiling on any predictor's gain.
Kill: oracle gain under 1.3x.
Guard: tail latency and per-query minimum recall constraints must be declared first.

A5. Memory-level parallelism
From: computer architecture, SSD index design.
Breaks: the assumption that in-memory search expands one node at a time.
Mechanism: expand the best W unexpanded candidates together, issuing their fetches before processing any, as SSD indexes do with beam width. On batch tracks, interleave several queries per thread so one query's miss overlaps another's work.
Prior art known: DiskANN beam width (SSD); KBest pipelined prefetch; VeloANN coroutine interleaving (SSD); AMAC for hash joins.
First test: hardware counters for last-level-cache misses and stalled cycles per query, with W in {1,2,4}, at matched recall on the largest dataset.
Kill: stalled-cycle share below 30% of query time.

A6. Address-translation audit
From: operating systems.
Breaks: the assumption that mmap is free. A file-backed mapping on 4KB pages can turn every random access into a TLB miss plus a page walk.
Mechanism: put hot structures on 2MB or 1GB pages. For multi-process embedded use, share them through a huge-page-backed memory object instead of a file mapping.
Prior art known: standard practice in Intel SVS and KBest. Included as a check, not a discovery.
First test: data-TLB miss counter per query on the benchmark host class.
Kill: fewer than one data-TLB miss per ten node visits.

A7. Decision-relevant precision
From: rate-distortion theory, psychophysics (just-noticeable difference).
Breaks: the assumption that every vector gets the same number of bits, allocated to minimise reconstruction error. Only bits that can flip a ranking at the top-k boundary matter.
Mechanism: extra refinement bits only for vectors whose local margin is smaller than their quantisation error. Sparse-region vectors keep the coarse code and never need a rerank fetch.
Prior art known: anisotropic quantisation (ScaNN), LVQ two-level codes, bit allocation in transform coding. Per-vector variable rate is less common.
First test: rung 2 counter. Fraction of rerank fetches that change the final top-k, and how concentrated those are in dense regions.
Kill: decisive rerank fetches spread evenly across density.

A8. Flow-reinforced graph refinement
From: biology (Physarum network formation), transport engineering.
Breaks: the assumption that a graph is finished when insertion and pruning end.
Mechanism: run many training searches, accumulate flow on edges that lie on successful paths, drop zero-flow edges, add a shortcut wherever searches repeat the same detour. Lower degree at equal recall.
Prior art known: ONNG path-based degree adjustment, GraSP learned sparsification.
First test: edge-usage histogram over training searches drawn from held-out base points.
Kill: fewer than 25% of edges unused.


B. FILTERING AND DYNAMIC SEARCH

B1. Block-max pruning for filtered search
From: web search engines (WAND, block-max indexes).
Breaks: the assumption that filtered vector search must choose between scanning a posting list and filtering a graph.
Mechanism: posting lists in blocks ordered by vector cluster, each block carrying a geometric lower bound on distance. Intersection skips on both identifier alignment and bound against the current k-th best.
Prior art known: ParlayANN IVF-squared (clustered posting lists).
First test: count blocks a safe bound would skip on the filter track's query mix.
Kill: skip rate under 50%.

B2. Local-selectivity planning
From: database query optimisation (correlated predicates).
Breaks: the independence assumption. Labels and vector position are correlated, so global selectivity picks the wrong plan.
Mechanism: per-cluster label sketches give selectivity near the query in constant time. Plan per query: unfiltered search with post-filter where matches are locally dense, posting scan where sparse.
Prior art known: NaviX adaptive-local, ACORN.
First test: ratio of local to global selectivity per query, and how often the best plan differs.
Kill: plans differ on under 10% of queries.

B3. Generational index
From: garbage collection (most objects die young).
Breaks: the assumption that every insert must be linked into the graph at once.
Mechanism: new points live in a flat, SIMD-scanned young generation and are promoted in batches. Points deleted before promotion never touch the graph, so they cost no repair.
Prior art known: FreshDiskANN delta index, LSM-style vector stores.
First test: from the runbooks, the fraction of points deleted within N operations of insertion.
Kill: under 10% across runbooks.
Note: see Z1. The streaming leader is at 0.99786 and the row's target is 1.

B5. Certified-exact search
From: operations research (branch and bound), numerical analysis (interval arithmetic).
Breaks: the assumption that approximate search can only be made more accurate, never proven exact. A recall target of 1 needs a proof per query, not a larger beam.
Mechanism: keep a deterministic lower bound on distance for every unvisited partition and every unrefined code. A query stops only when its k-th best distance is below every remaining bound, so its result is exact with a certificate. Cost concentrates on the few queries whose bounds are loose.
Prior art known: VA-file (exact search over approximations), ball-tree and partition bounds, RaBitQ error bounds (probabilistic, not deterministic).
First test: rung 2 counter. Fraction of queries certified exact at the current budget, and the work needed to certify the rest.
Kill: certifying the uncertified queries costs more than the track's time limit allows.

B4. Far-field batch insertion
From: computational physics (Barnes-Hut and tree codes for N-body simulation).
Breaks: the assumption that each insert runs its own search.
Mechanism: cluster a batch. One search per micro-cluster supplies the shared far-field candidates. A dense local all-pairs join inside each cluster supplies the near field.
Prior art known: NN-descent local join, GPU batch builders, ParlayANN batch insert.
First test: overlap of candidate sets between inserts in the same micro-cluster.
Kill: mean overlap under 50%.


C. RETRIEVAL AND ANSWER QUALITY

C1. Belief ledger with executable queries
From: accounting (ledgers), event sourcing, bitemporal databases.
Breaks: the assumption that memory hands text to a reader and the reader does the arithmetic.
Mechanism: every state change is a dated ledger entry with provenance. Relative dates are resolved at write time. The model plans a small query (count, latest, before, between). A deterministic engine executes it. The reader verbalises a computed answer with its source turns.
Prior art known: Zep and Graphiti (bitemporal graph), TReMu (neuro-symbolic temporal reasoning), Mastra OM (dated observations).
First test: classify current errors by type. Count those caused by date arithmetic, counting or stale values with correct evidence retrieved.
Kill: under a third of remaining errors.
Guard: report it as a system change, separately from reader, judge and context budget.

C2. Capture-recapture completeness
From: ecology and epidemiology (population estimation).
Breaks: the assumption that top-k is the right contract for counting questions. A count needs every instance and a way to know when all are found.
Mechanism: two independent retrievers. The overlap of their hits estimates the total number of relevant memories (n1 times n2 over the overlap). Keep retrieving until found matches estimated.
Prior art known: recall estimation in legal e-discovery and systematic reviews. Not known to the author in agent memory.
First test: on aggregation questions with gold evidence, compare the estimate with the true count.
Kill: estimate error above the miss rate it is meant to detect.

C3. Proof-carrying answers and absence proofs
From: programming languages (proof-carrying code), forensics (chain of custody).
Breaks: the assumption that the judge is the first check an answer meets.
Mechanism: the reader must cite spans. A deterministic checker verifies that the spans exist and contain the stated values and dates. A failed check triggers re-retrieval or abstention. For abstention, an exhaustive cheap scan of the user's whole store proves absence, which is affordable because each store is small.
First test: fraction of current wrong answers whose cited or retrieved evidence does not contain the answer.
Kill: under 20% of wrong answers.

C4. N-version memory
From: fault-tolerant avionics.
Breaks: the assumption that extraction at write time is trustworthy enough to discard the raw record.
Mechanism: keep the raw turn and the extracted fact as independent versions. Disagreement at answer time forces a raw re-read.
First test: rate of extraction errors among current failures.
Kill: under 10%.