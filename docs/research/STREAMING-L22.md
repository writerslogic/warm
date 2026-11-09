# Streaming L22: partition tier (IVF + 4-bit LUT16 scan + exact reorder), design and kill tests

Date 2026-11-07. Design only: nothing in this note was run on writerslogic-03, no code was written, no spend.
Tags: [M] measured (file:line or ledger entry), [P] published, [I] inferred, [C] conjecture.
Code paths are in /Volumes/A/.hms-wt-levers (4016fdf0) unless stated. Harness paths are under /Volumes/A/.hms-lb/bigann. Board and runbook facts are in STREAMING-L10-DESIGN.md Part 1 and are not repeated.

## 0. Why L22 now (the L10 verdict)

- L10 stage 1 as prototyped is killed on recall, not time [M]: bigann/wl03-stage1c/out/cmp.txt: stage1 recall 0.7649 vs pin 0.8768 (-0.112, gate >= -0.005), worst per-search -0.236; wall 3,583 s with insert 892 + delete 1 + scan 74 = 967 s of update time (the time diagnostic would have passed) and 2,690 s of build-kernel search.
- The msturing-prefix isolation (bigann/wl03-prefix-iso/NOTES.md) [M] puts 0.057-0.084 of the gap in construction alone, with no deletes, at 1M and 3M: single insertion pass, no upper layers, `slots[0]` entry, exact-f32 search without codes/rerank. Insertion order (+0.014) and capacity (-0.013) are minor. Closing it means reproducing the pin's two-pass build plus medoid/upper layers plus codes+rerank search inside `MutableBase`, which was already the L10 stage-2 scope before any delete/repair work: weeks, for a ceiling at or near the pin's 0.877 (the open graph entries at R32 sit at 0.87-0.92, LEVERS-2026-11-06.md section 1 rank 9 and 2.2 L15).
- L22 has the only "supported" verdict in the lever review and a board existence proof: scann 0.9924 corrected, inside 1 h, on this runbook [P, bab/neurips23/streaming/res_final_runbook_AzureD8lds_v5.csv line 22; LEVERS section 1 rank 9]. The flagged "indexsize 1024" is a harness artifact (the harness measures the empty setup object, as for pinecone and pyanns) [LEVERS L22 row, verifier note], so it does not discount the recall; it does mean memory must be measured by us.
- Decision recorded in STATE.md (2026-11-07, maintainer delegated): the pre-registered KILL of L10 stage 1 stands; `MutableBase` (update/mutable.rs, 791 lines, tests in update/mutable/tests.rs) is kept as a measured negative result and is not extended. L22 starts with the kill tests in section 6; no runs before STATE.md registers them.

## 1. The existence proof, read from source [P]

bab/neurips23/streaming/scann/scann.py and config.yaml (msturing-30M-clustered, run group dynamic4M):
- Brute force (`score_brute_force`, `reserve(max_pts)`) until `size() > 4,000,000`, then one `rebalance(config)`; after that `upsert` and `delete` are incremental, no further rebuild. Live rows first reach 4M at search step 47 of 640 (section 2), so 46 search steps ran brute force.
- Partitioning: 5,000 children, k-means on SquaredL2, 10 iterations, min cluster 10, `expected_sample_size` 1,000,000, random init; query spilling FIXED_NUMBER_OF_CENTERS 700 (`leaves_to_search`); database spilling TWO_CENTER_ORTHOGONALITY_AMPLIFIED (a row may live in two lists).
- Codes: asymmetric hashing, CHUNK projection, 50 blocks of 2 dims, 16 clusters per block (4-bit PQ, 25 B per code), residual quantization on, noise shaping (anisotropic loss) threshold 0.2, codebook sample 32,000, `lookup_type: INT8_LUT16` (the 4-bit LUT16 scan with 8-bit LUT entries), global top-n.
- Exact reorder of 317 (`pre_reorder_num_neighbors`), `final_num_neighbors` 10, batch 1,250, 8 threads.
- Metric caveat [P, unresolved]: the adapter builds the searcher with `'dot_product'` and `DotProductDistance`, while the dataset is Euclidean (datasets.py `distance()`; our GT is exact L2). Partitioning and quantization use SquaredL2 and only the ranking distance is dot product. The 0.9924 therefore says that MIPS ranking matched the L2 ground truth on msturing to 99%, which holds only if row norms are nearly constant [C, not checked; the base file is on wl-03]. WARM's port uses L2 throughout; kill test A measures the L2 ceiling directly, so this caveat cannot leak into our numbers.
- Also relevant [P, LEVERS L22 row]: Quake (PR 336) finished inside 1 h at 0.854 with adaptive partition maintenance; SPFresh/LIRE (2410.14452) and Ada-IVF (2411.00970) give the split/merge maintenance pattern. None of them is needed for the first kill test.

## 2. Runbook profile that sizes the design [M, computed from bab/neurips23/runbooks/final_runbook.yaml, stdlib parse]

1,280 ops: 29,998,834 rows inserted, 27,630,980 deleted, 640 searches of 10,000 queries (testQuery10K.fbin). Live rows at the search steps: first search 38,806; search 10: 177,161; search 50: 4,134,502; max 10,292,043; mean over the 640 searches 5,021,822; sum 3.214e9 row-searches; quantiles (10/25/50/75/90%) 2.42M / 3.37M / 4.89M / 6.58M / 7.90M. Live first reaches 200k at search 11 (448,898 rows inserted so far), 500k at search 12 (733,493), 1M at search 16 (1,239,816), 2M at search 26, 4M at search 47 (4,103,827 inserted). 15 searches run below 1M live, 209 below 4M.

Consequences: a brute-force phase to 4M as scann did costs sum(live over the first 46 searches) x 10k queries x 200 FLOP = 7.88e7 x 1e4 x 200 = 1.6e14 FLOP, 800-1,600 s at 100-200 GFLOP/s on 8 cores [I]; training the coarse quantizer at 1M live (search 16; the first 15 searches sum to 4.73M live rows, 9.5e12 FLOP, 50-100 s) removes most of it. The clustered stream means the 1M prefix is not a sample of the 30M distribution; this is the central recall risk (section 5).

## 3. What WARM already has [M]

| Asset | Where | Reusable as is | Gap |
|---|---|---|---|
| 4-bit LUT16 block scan | qgraph/fastscan.rs: `BLOCK = 32`, `scan32(codes, lut, m, out)` for any `m`, u16 accumulation for `m <= 256`, NEON and AVX2 kernels plus a scalar fallback (lines 13-68) | yes: the inner kernel for both PQ-50 (`m = 50`) and per-dim scalar 4-bit (`m = 100`) | no block container, no per-list LUT builder, no admission threshold loop |
| Centroid-relative scalar IVF | qgraph/ivf.rs `QuantizedIvf` (static build from external centers and assignments, 4/8-bit codes, bounds pruning, `IvfSearcher` with nprobe/rerank) | code pattern only | angular and unit-vector only (`validate_units`), inner product, static (no insert/delete), no retained vectors, no LUT16 layout (codes are i8 dot products via `dot_i4_inline`) |
| Id-to-slot table | update/mutable.rs `IdTable` (2^16-entry u32 pages, lazily allocated, PAGE_BITS 16) | yes: id -> location | location needs (list, position) instead of a slot |
| k-means | filter.rs:974 (i8 domain, per-label lists, `kmeans_iters` 6); repair.rs:415 `kmeans_medoids` | neither directly | an f32 Lloyd's k-means for 5,000 centers over a 256k-1M sample with a blocked distance kernel |
| 4-bit scalar quantizer | vertex.rs:729 `quantize4` (per-vector scale, codes -7..7 packed two per byte) | no | LUT16 needs a grid shared by every code a LUT scores (per-list per-dim grid or a shared PQ codebook); a per-vector scale cannot be folded into a LUT |
| Streaming adapter | bigann/wt-streaming/neurips23/streaming/warm/warm.py (`setup(dtype, max_pts, ndim)`, `insert`, `delete`, `replace`, `query`, `set_query_arguments`); `max_pts` is currently dropped | yes, with a `mode` key | a `PartitionIndex` binding beside `UpdatableIndex`/`MutableIndex` in src/bindings/python/vector_index.rs |
| L2 kernels | kernels.rs `dot` and friends; f32 rows resident pattern from `MutableBase.rows` | yes | a batched query-by-centers distance (10k x 5,000 x 100 per step) wants a blocked kernel, otherwise it is 3e9 FLOP per step of scalar loops |

## 4. Design: `PartitionTier`

One new module, `src/core/qgraph/update/partition.rs` (plus `partition/tests.rs`), independent of `UpdatableVGraph` and `MutableBase`, feature `experimental` until the kill tests pass.

Data (sized by capacity C = `max_pts`, 10,292,043 on this runbook; ids are dense in [0, 30M)):
- `rows: Vec<f32>`, C x dim, slot-addressed, written once per insert; the reorder set. 4.12 GB at C x 400 B.
- `norms: Vec<f32>` per slot (Euclidean), 41 MB.
- `slot_to_id: Vec<u32>`, free-slot heap (lowest first, as `MutableBase.free`), `next_fresh`.
- `IdTable` id -> slot (120 MB at 30M ids) plus per slot `(list: u16, pos: u32)` (6-8 B, 82 MB) so a delete finds its code in O(1).
- Coarse centers: `centers: Vec<f32>` 5,000 x dim (2 MB), `center_norms`.
- Per list: `codes: Vec<u8>` in 32-code LUT16 blocks (`16 * m` bytes per block: 800 B for PQ-50, 1,600 B for scalar-100), `ids: Vec<u32>` (slot per code, parallel to the block order), `len`, and the list's LUT parameters (PQ: none, codebook shared; scalar grid: per-dim `lo`, `step` as f32, 800 B). Mean list at 10.29M live: 2,058 codes = 65 blocks.
- Codebook (PQ variant): 50 blocks x 16 centroids x 2 dims f32, trained once on 32k residuals (scann's sample size) when the centers are trained; shared by every list, so LUTs are per (query, list) because the residual depends on the list's center.

Operations:
1. **Train** (once, when live first reaches `train_at` rows, default 1,000,000): Lloyd's k-means, 5,000 centers, 10 iterations, on a uniform sample of 256k-1M live rows (sample size is a knob; scann used 1M), random init; then (PQ variant) k-means of 16 centroids per 2-dim block on 32k residuals; then assign and encode every live row and build the lists. Cost: 10 x 1M x 5,000 x 100 x 2 = 1e13 FLOP = 50-100 s at 100-200 GFLOP/s [I]; one-off. Before training, search is exact brute force over the resident f32 rows (<= 1M rows, 15 steps, 150-300 s total [I]).
2. **Insert** (batch): nearest center per row (a batched rows-by-centers L2, 5,000 x 100 x 2 FLOP per row; 30M rows = 3e13 FLOP = 150-300 s over the run [I]); take a slot, write the f32 row, encode the residual, append the code to the list's current tail block and the slot to `ids`, record `(list, pos)`. No graph, no routing tree, no repair.
3. **Delete**: O(1) swap-remove: copy the list's last code into the hole (50 or 100 nibble moves inside two blocks), move its slot entry, fix that slot's `(list, pos)`, free the slot. 27.6M deletes = well under 60 s [I]. No tombstones in the scan path, so no dead-fraction widening and no scans.
4. **Search** (a step of Q = 10,000 queries, k = 10), list-major:
   a. center scores: Q x 5,000 L2 distances (1e10 FLOP per step, 6.4e12 over the run: 30-65 s at 100-200 GFLOP/s [I]); per query the 700 nearest centers (`nprobe`).
   b. invert: for each list, the queries that probe it (mean Q x 700 / 5,000 = 1,400 per list); process lists in parallel across 8 threads, each list's codes (100-200 KB) staying in L2 while its 1,400 queries are scanned.
   c. per (query, list): build the LUT (PQ: 50 x 16 entries of `|q_b - c_b - codeword|^2`, quantized to u8 with a per-query scale and offset, as INT8_LUT16; scalar grid: 100 x 16 entries of `(q_j - c_j - lo_j - v*step_j)^2`), then `scan32` over the list's blocks; admit a code to the query's top-317 heap only when its u16 score beats the query's current 317th (threshold compare before any heap work).
   d. reorder: for each query, exact L2 over the resident f32 rows of its 317 candidates (3.17k x 100 x 2 FLOP plus 317 random 400 B reads), top-10.
   Scan cost: per step the scanned codes are 0.14 x live x Q; over the run 0.14 x sum(live over searches) x 1e4 = 0.14 x 3.214e9 x 1e4 = 4.5e12 code scores; `scan32` at about 1.5-2 cycles per code per 50 groups on AVX2 [I, FAISS-class LUT16 throughput; unmeasured on WARM] is 1.5-2e9 codes/s per core, 1.2-1.6e10 on 8 cores: 280-380 s for PQ-50, twice that for scalar-100. Reorder: 6.4e6 queries x 317 rows = 2.0e9 row reads, latency-bound at 30-100 ns with batching: 60-200 s [I]. Memory traffic is not the bound once list-major: each step reads the live codes once (0.26 GB at 25 B), 165 GB over the run, seconds at DRAM speed; query-major scanning would read 36 MB per query and be DRAM-bound (the LEVERS "about 1,800 s at 60 GB/s" warning).
5. **Maintenance** (stage 2, only if kill test B shows drift): split a list when its length passes `split_at` x mean (Quake/SPFresh pattern: 2-means over its members, new center, re-encode the members, both lists keep their sizes bounded); merge a list below `merge_below` x mean into its nearest center's list. A split re-encodes 2 x mean codes and touches only the ids and `(list, pos)` of those rows. Not in stage 1.
6. **Durability**: none for the board run (the harness never restarts the index). Stage 3 if the tier becomes a product surface: checkpoint rows/codes/ids plus an op log, as `UpdatableVGraph` does (update.rs:727-770).

Invariants (property tests on random op sequences, fingerprint after replay):
- P1 every live id has exactly one `(list, pos)` and `ids[list][pos]` names its slot; every code position of every list names a live slot (no tombstones exist).
- P2 `sum(len)` over lists = live rows; free slots + live = `next_fresh`.
- P3 the swap-remove preserves P1 for the moved row.
- P4 deterministic: same op sequence, same sample seed -> identical centers, codes, lists and results.
- P5 search over the lists with `nprobe = 5,000` and reorder = live rows returns the exact brute-force top-10 (the scan admits everything, the reorder is exact).
- P6 INT8 admission: the running threshold T is converted into each (query, list)'s quantized units, (T - sum of block minimums) / scale, before the u16 compare; on fixed centers, int8 and float LUT recall agree within 0.001 (test A's cross-list merge bug, STATE.md 20:45Z).

Memory at C = 10,292,043, dim 100 [I, from the sizes above]: rows 4.12 GB + norms 0.04 + slot_to_id 0.04 + IdTable 0.12 + (list, pos) 0.08 + codes 0.26 (PQ-50) or 0.52 (scalar-100), with 1/32 block slack + list growth slack of up to 2x on `Vec` doubling (bound it by reserving `mean x 1.5` per list after training and growing in 32-code blocks: codes stay <= 0.6 GB) + ids 0.04 + centers/codebook < 0.01 = 4.7-5.0 GB, plus the harness about 1 GB [I, as in L10 2.5] = 5.7-6.0 GB against the 8 GiB cap. Database spilling (two lists per row, as scann) would add one more code and id per row (+0.3 GB) and is affordable; it is a stage-2 recall knob, not a stage-1 default.

Time budget, Zen 4 seconds [I unless marked]:

| Phase | Optimistic | Central | Pessimistic |
|---|---|---|---|
| Brute force before training (15 steps, 4.73M row-steps x 10k queries) | 50 | 100 | 200 |
| Train (k-means 5,000 x 10 iters on 1M, codebook, encode 1M) | 40 | 90 | 200 |
| Insert 30M (assign + encode + append) | 120 | 250 | 450 |
| Delete 27.6M (swap-remove) | 10 | 30 | 60 |
| Search 625 steps after training (centers + LUT16 scan + reorder) | 350 | 650 | 1,100 (scalar-100 or poor batching) |
| Total | about 570 | about 1,120 | about 2,010 |

The pin spends 4,816 s in consolidations and 6,175 s total [M, L10 2.2]. L22 removes the graph entirely, so the 78% consolidation share disappears by construction rather than by tuning; the pessimistic column at 1.8x (3,620 s) does not fit the 1 h rule on a VM 1.8x slower than Zen 4, the central one (2,020 s) does; so the pessimistic search case must be avoided: PQ-50 and list-major batching are requirements, not options. Hardware transfer stays an inference until a reference-VM run.

## 5. Risks, ranked

1. **Distribution drift on the clustered stream** [C, the main one]. Centers trained on the first 1M rows describe the clusters present early; later inserts fall into few lists, lists grow unevenly, probe recall drops. scann trained at 4M and did not rebalance again, yet scored 0.9924 [P], which bounds the damage for a 4M-trained quantizer under a 14% scan fraction; a 1M-trained one is untested. Kill test B measures recall per step; the stage-2 answer is list splitting (section 4 item 5) or training at 2M (search 26) at the cost of 11 more brute-force steps.
2. **Quantizer choice**. PQ-50 with 16 centroids per 2-dim block (scann's) halves the code bytes and the scan time relative to a per-dim 4-bit grid, but needs a training stage and a residual codebook that must stay valid under drift; a per-list per-dim grid needs no training but 2x the scan work and re-gridding when clamping grows. Kill test A runs both on the same lists.
3. **LUT precision**: INT8 LUT entries summed in u16 over 50-100 groups lose precision for far candidates, which is harmless, and for near-threshold admissions, which the 317-wide reorder absorbs (scann's margin: 317 reordered for k = 10). Measured in kill test A as the gap between exact-residual scoring and LUT16 scoring at equal nprobe.
4. **Metric transfer**: our L2 vs scann's dot-product ranking (section 1). Kill test A settles the L2 ceiling with no quantization at all.
5. **Memory**: 5.7-6.0 GB projected against 8 GiB [I]. `Vec` doubling on 5,000 lists is the only unbounded term; reserve after training and grow by blocks. Measure cgroup peak in kill test B; an OOM is a fail.
6. **Search batching complexity**: the list-major loop with per-query thresholds is the only non-trivial code; a query-major fallback is 10x slower on DRAM traffic and must not be what ships.

## 6. Kill tests (pre-register in STATE.md before launch; compute only, no spend)

Test A, static recall ceiling on wl-03 (existing data: `~/bigann/piso/gt_1000000.npy`, `gt_3000000.npy`, the 30M base file, queries 0..999; numpy in the pin venv, one script, no WARM code). Script: bigann/wl03-l22a/scripts/l22a.py (chain_l22a.sh, status_l22a.sh); registered and launched 2026-11-07 20:01Z (STATE.md), outputs under `~/bigann/piso/l22a/out/`:
1. k-means 5,000 centers, 10 iterations, on a 256k uniform sample of the prefix (and a second arm trained on rows 0..1M only, to imitate the streaming prefix); assign the N rows.
2. Partition ceiling: recall@10 of exact L2 restricted to the union of each query's 700 nearest lists (nprobe 700 of 5,000, scan fraction 14% as scann), at N = 1M and 3M. Also nprobe 350 and 1,400 to see the slope.
3. Quantized recall: the same with (i) PQ-50/16 residual codes with an exact-float LUT, (ii) the same codes with INT8 LUT entries (u8 scale per query), (iii) per-dim 4-bit grid codes with INT8 LUTs; candidates = top 317 by LUT score, then exact reorder, recall@10.
Gates: ceiling >= 0.98 at both N (otherwise the 14% fraction does not transfer to L2 msturing: STOP, L22 dead at this scan fraction; try nprobe 1,400 once for the record); quantized (ii) or (iii) >= 0.95 at both N with reorder 317 (the pin is 0.835/0.842 at ef 256 on these prefixes [M, NOTES.md]). Caps: 3 h wall, 150 tool calls, `taskset -c 0-7 nice -n 19` under `flock ~/hms-timing.lock`, nothing written outside `~/bigann/piso/l22a/`. Cost model: assignments at 3M x 5,000 x 100 = 3e12 FLOP per arm (minutes with BLAS); the LUT16 emulation in numpy scores 1,000 queries x 0.14 x 3M codes = 4.2e8 code-scores, fine.
Expected before the run [I]: ceiling 0.97-0.99, PQ-50 0.93-0.97. Measured (2026-11-07, STATE.md 20:45Z) [M]: no-spill ceiling at nprobe 700 = 0.9115 (1M) / 0.9333 (3M); nprobe 1400 0.965 / 0.977; GATE FAILED. Prefix-trained quantizer (rows 0..1M) at 3M: -0.011 vs sample-trained (drift check passed). pq50_float with reorder 317: 0.913 / 0.928 (500 queries; same-query ceiling comparison pending). The int8 arms of test A were invalid (per-(query, list) LUT scales merged without de-quantizing). Test A2 (database spilling with the SOAR loss, scann's omitted component; same 0.98 gate) PASSED (STATE.md 21:00Z) [M]: SOAR lambda 1.0 spill ceiling at nprobe 700 = 0.9835 (1M) / 0.9917 (3M); at equal codes scanned spill@700 beats no-spill@1400 by +0.019 / +0.015; PQ-50/16 with INT8 LUT16 (de-quantized per (query, list)) and reorder 317 = 0.9796 / 0.9894, within 0.002 of the same-query ceiling. Consequences for section 4: spilling (one SOAR second list per row, lambda 1.0) is a stage-1 default, not stage 2; codes per row double (scan fraction ~28% at nprobe 700, +0.3 GB memory, 6.0-6.3 GB projected with harness); search central ~1,300 s and run total ~1,700 s on Zen 4 [I]; nprobe 500 is the first time lever (3M ceiling 0.9838). Script bigann/wl03-l22a/scripts/l22a2.py.

Test B, streaming replay (after A passes and the Rust `PartitionTier` exists): the final runbook on wl-03 with the kill-test discipline of L10 2.9 (pin gate 0.876838578125 first, same cpuset, 8 GiB scope, sequential arms), config `mode: partition, lists: 5000, nprobe: 700, reorder: 317, train_at: 1000000, codes: pq50, spill: soar, soar_lambda: 1.0` (spilling per A2; A3 measured the fixed-quantizer ceiling at 0.9875-0.9980 over the stream at 0.42-0.44 of live scanned). Gates: mean recall over 640 searches >= 0.90 (above the pin by 0.023, and above pyanns 0.8702/0.8866 whichever the live board uses); no single search step below 0.80 after training; wall < 3,000 s; cgroup peak < 8 GiB; `WARM_MODE partition` line printed. Pre-registered diagnostic: a recall trend that falls with step index after training is drift (risk 1) and sends the design to stage 2 (splits or `train_at` 2M) before any other change; a flat but low recall is quantization or nprobe (raise reorder to 500 and nprobe to 1,000 once, time permitting, and record both).

## 7. Staged plan

- Stage A (1 day): kill test A as above, numpy only. Decides whether any Rust is written.
- Stage 1 (1-2 weeks): `PartitionTier` with PQ-50 codes, list-major LUT16 search, swap-remove deletes, brute force before `train_at`, Python binding `PartitionIndex`, adapter `mode: partition`; property tests P1-P5; a local 1M/3M prefix check reproducing test A's numbers from Rust (same centers loaded from the numpy run, so the comparison isolates the kernels).
- Stage B (1 day of wl-03 compute): kill test B.
- Stage 2 (1 week, only if B shows drift or a recall shortfall): list split/merge maintenance, database spilling, `train_at` and nprobe/reorder sweeps (reported as a sweep, not a point).
- Stage 3 (as needed): checkpoint/log, docs, results JSON, the reference-VM run per STATE.md:43.

Out of scope for L22: any change to the graph index, `MutableBase`, or the consolidation path; the G05 Optimist routing (conditional lever, after B); any VIBE or filter use of the tier.


## 8. Measured operating point (2026-11-07, STATE.md 21:00Z-23:30Z)

- Recall [M]: A2 (prefixes) SOAR lambda 1.0 spilling lifts the L2 ceiling past 0.98 at nprobe 700 and beats no-spill at ~equal codes; INT8 LUT16 with per-(query, list) de-quantization is lossless vs float (<= 0.0006); raw-vector codes lose 0.004-0.009 vs residual and are rejected. A3 (actual stream, official per-step GT): a quantizer trained once at op 32 keeps the spilled ceiling at 0.9875-0.9980 (nprobe 700) and 0.966-0.988 (nprobe 350), within 0.002 / 0.007 of a per-step retrain, but its lists skew so a query scans 0.43 of live at nprobe 700 instead of 0.28.
- Time [M bench on wl-03 cpus 0-7, I projection]: one step at mean live costs 2.20 / 2.99 / 4.31 s at nprobe 350 / 500 / 700 (scan 4.8-5.2e9 codes/s, per-(query, list) table builds 0.87-1.45 s per step, independent of live). Projected search: 2,230 s at nprobe 350 with fixed centres, 2,080 s at nprobe 500 with balanced lists, against the registered 2,300 s gate; nprobe 700 does not fit. The section-4 time table (central ~1,100-1,700 s Zen 4) was optimistic by ~2x on the scan rate and omitted the per-step table cost.
- Candidate stage-1 configuration (pending the non-search measurement registered in STATE.md 23:40Z, which decides whether list maintenance is in stage 1): 5,000 lists, train_at 1M (op 32), SOAR lambda 1.0 spill, PQ-50/16 residual codes, INT8 LUT16 per (query, list) with P6 threshold conversion, nprobe 350, reorder 317, fixed centres. Stage 1.5: list maintenance (balanced lists save ~600 s at nprobe 350 or allow nprobe 500). Kernel reserve: integer table composition, multi-query block loads.
- Non-search measured (STATE.md 23:55Z) [M]: 2,090 s with numpy kernels (insert nearest + SOAR spill 1,428 s, pre-train brute force 539 s, training 93 s, deletes 30 s assumed), so the total exceeds 3,000 s in every configuration. The time case now rests on faster assignment (restricted spill, hierarchical centres) and a blocked brute-force kernel, each still to be measured; PartitionTier is not started.

## 9. Non-search kernels, maintenance and acceptance (2026-11-08)

Every run below was registered before launch and ran on wl-03 (Intel Core Ultra 9 185H, cpus 0-7 = 4 P-cores x 2 HT) under the timing lock. Scripts are in `benchmarks/research/l22_partition/scripts/` and outputs in `benchmarks/research/l22_partition/results/` (`a_out*` = numpy studies, `bench_run*` = Rust bench and acceptance). [M] = measured, [I] = inferred or projected.

Assignment and brute force (`l22b.py` -> `a_out6`, `l22b2.py` -> `a_out8`) [M]:
- On 1M rows, the GEMM alone takes 4.4 s and the full numpy nearest + SOAR spill takes 46.9 s; the elementwise epilogue costs more than the GEMM.
- Exact restricted spill with a certificate (SOAR loss >= L2) fails: the certificate holds on only 0.5-2.5% of rows at lambda 1.0.
- Restricted spill without fallback (top-32 by L2, SOAR among the other 31, no second matmul) is ADOPTED. Over 8 stream ops and the 1M/3M prefixes, the worst recall change is -0.0001 and codes scanned rise at most 1.9%. Sustained mean 11.75 s per 1M rows (4 runs, 10.2-13.4).
- Hierarchical assignment (71 super-centres, m = 4 or 8) is REJECTED: ceiling -0.005 to -0.011 and 5-12% more codes scanned.
- Threaded brute force with a running-threshold filter takes 16.8 s per step at 1.24M live x 10k queries (numpy 141 s), with recall 1.00000 vs the official GT.
- Training earlier (op 22 / 24) is REJECTED: op 22 loses -0.0089 recall at op 100, and op 24 scans 1.20x the codes at op 32.

List maintenance (`l22c_maint.py` -> `a_out7`, stopped at op 400) [M]: splits at T x the current mean, without neighbour reassignment, need nprobe >= 700 (T = 3) or never reach the fixed arm's nprobe-350 recall (T = 2, which runs away to 30,147 lists). Both are REJECTED; stage 1 has no maintenance.

Total-time rule (`project_total.py`, sustained bench means in `bench_run5`): at nprobe 350 with fixed centres, search 2,198 s (tables 490 + scan 1,558 + 150 [I]) plus non-search 485 s (k-means 38, train assignment 15, insert assignment 338, pre-train brute force 64, deletes 30 [I]) gives 2,682 s, which passes the 3,000 s gate with 318 s spare. This is a projection; the stream has not been run end to end. The Rust measurements below supersede it: the bench's scan rate was not reached on real lists.

PartitionTier (`src/core/qgraph/partition.rs`, `experimental`) A2 prefix acceptance (`bench_run6`) [M]: recall@10 at nprobe 350 / 700 is 0.9496 / 0.9802 at 1M (A2 0.9494 / 0.9794) and 0.9676 / 0.9880 at 3M (A2 0.9694 / 0.9894), within the 0.003 gate. A 10k-query step on the 3M prefix takes 2.38-2.42 s, against the projection's ~1.9 s and the pre-check gate of 2.3 s. The search path is profiled before the full-stream test B.

PartitionTier search profile and live sweep (`bench_run8` to `bench_run11`, 10k queries per step, RAYON 8) [M]:
- 3M prefix with prefix-trained centres (`bench_run8`, `bench_run9`): step 2.22-2.41 s at nprobe 350. Table builds take 0.79-0.82 s, as projected. The code scan runs at 3.5-3.7e9 codes/s against the uniform bench's 4.75-5.2e9, and thread imbalance (slowest thread minus mean) adds 0.10-0.18 s.
- Stage-1 state (`bench_run10`, `l22f_prep.py`): op-32 centres trained by A3's procedure, the real live rows at ops 200 / 1172 / 648 (2,999,384 / 6,001,880 / 9,990,953 rows), and official step GT.
  - At nprobe 350 a query scans 0.227-0.238 of live (A3's 0.23). The step takes 3.0-3.2 / 4.6-5.1 / 7.2-7.6 s, with recall@10 of 0.9712 / 0.9786 / 0.9862.
  - The code-scan rate stays at 3.5-4.1e9 at the longer lists. Admitted candidates (3.9-4.6k per query) do not drive cost.
  - The fit is step = 1.23 s + 0.62 s per 1M live. Whole-run total: 625 a + b x 3.209e9 + 485 s non-search = ~3,240 s, which FAILS the 3,000 s rule.
- nprobe sweep on the same states (`bench_run11`):

  | nprobe | recall@10 at 3.0M / 6.0M / 10.0M | fitted total |
  |---|---|---|
  | 150 | 0.9208 / 0.9362 / 0.9548 | 1,876 s |
  | 200 | 0.9396 / 0.9542 / 0.9670 | 2,170 s |
  | 250 | 0.9542 / 0.9644 / 0.9762 | 2,573 s |
  | 350 | 0.9712 / 0.9786 / 0.9862 | 3,157 s |

  The pre-registered rule takes the smallest nprobe with recall >= 0.95 at all three states and a total <= 2,850 s, which selects nprobe 250 for stage 1.
  - These totals are projections from a linear fit inside the 3-10M range, which covers the run's live quantiles of 2.42-7.90M.
  - Searches below 3M live may have lower recall.
  - The stream has not been run end to end; test B does that at nprobe 250.
  - Dynamic thread scheduling (imbalance 0.1-0.4 s per step) and multi-query blocking (the scan rate) are held in reserve.

Test B, the full stream through the harness (`results/testb_run1`, `testb/`) [M]:
- **Setup.** The `PartitionIndex` binding (features `python,experimental`) was driven by the PR 357 adapter with `testb/partition-mode.patch`, selected by `WARM_MODE=partition`.
  - Parameters: nprobe 250, reorder 317, 5,000 lists, train_at 1M.
  - It ran the stage-0 command (`run.py --nodocker`, streaming track, msturing-30M-clustered, final_runbook) on wl-03 cpus 0-7 in an 8 GiB scope with no swap.
- **Run.** Valid: 1,280 steps, exit 0, 640 searches.
  - The tier trained at search 16 with 1,239,816 rows live, as specified.
  - The 15 searches before it ran brute force at recall 1.0.
- **Recall@10.** The mean over 640 searches is 0.9651, against 0.8768 for the pinned graph entry on the same host and harness. The lowest post-train search is 0.944.
  - Means over 80-search bins run 0.958-0.972, with no downward trend, so the fixed op-32 centres did not drift into a recall loss.
- **Time and memory.** Wall time, as the sum of step seconds, is 2,276 s: search 2,105, insert 141, delete 30. Peak RSS is 6.45 GiB. Every registered gate passed.
- **Against the 2,573 s projection.**
  - Post-train partition search took 1,865 s, against 2,088 s projected.
  - Non-search work took 205 s, against 485 s.
  - Harness overhead, not projected, was about 206 s.
- **Scope.** This is one run on wl-03 (Core Ultra 9 185H, 8 threads on 4 P-cores). It is not the board's evaluation VM.
  - The board configuration selects partition mode from a `config.yaml` key (`testb/partition-mode-config.patch`). It needs its own run on a public commit before any submission claim.

s2b, the board configuration on the public pin (`results/s2b_board_config`) [M]:
- **Setup.**
  - Partition mode is selected by the `config.yaml` key `"mode": "partition"`, with `WARM_MODE` unset (`testb/partition-mode-config.patch`, now commit 56fc6e6 on the PR 357 branch).
  - The wheel was built from a `git archive` of main `e29b62fc`. Parameters: nprobe 300, reorder 317, 5,000 lists, train_at 1M, with the LPT query balancer.
  - Same command, host and scope as test B.
- **Run.** Valid: 1,280 steps, exit 0, 640 searches. The tier trained at search 16 with 1,239,816 rows live.
- **Recall@10.** The mean is 0.9726. The lowest post-train search is 0.9543. Means over 80-search bins run 0.9669-0.9780.
- **Time and memory.** Wall time (sum of step seconds) is 2,550 s: search 2,353, insert 160, delete 36. Process elapsed is 42:31. Peak RSS is 6.45 GiB. Every gate passed.
- **Against expectation.** The run10 step fit put nprobe 300 at about 2,800 s with non-search at 485 s. The measured 2,550 s is under that, as in test B.
- **Scope.** One run on wl-03, not the evaluation VM. The PR 357 description (`pr357-body.md`) carries the same numbers and that caveat.


Ice Lake, the evaluation VM's class (GCP n2-custom-8, Xeon 2.6 GHz; `results/il_s2*`) [M]:
- **The pin fails on time.** The pinned `e29b62fc` config takes 5,618 s, 2.2x the Core Ultra time and over the hour. Block and admitted counts are identical to the Core Ultra run.
- **Profile.** The scalar threshold-compare loop took 41% of search, more than the LUT16 kernel's 36%. This also explains why PQ-25 cut the scan by only 12% in run14.
- **Fused scan, `5bba104f` (merged in #403).** The fix has three parts:
  - in-register admission on the u16 sums;
  - an AVX-512 table build;
  - a VBMI+VNNI quad layout, where four queries share each block's unpack.
  Recall is identical and Ice Lake search drops 2.96x, to 2,187 s total. On wl-03 (AVX2, no quad layout) the same commit is 1.18-1.44x faster at identical recall, blocks and admitted counts (run15).
- **The headroom went to recall:** nprobe 350 / reorder 1000 gives 0.9808 in 2,466 s, and nprobe 400 / reorder 1000 gives 0.9848 in 2,666 s. That config was submitted first, pinned at `459a5403`.
- **Second scan round, `c5cd1c61` (merged in #406).** Eight queries share each block. The live set is recomputed only when a threshold changes. The next chunk's tables are prefetched, and the exact reorder is batched. On Ice Lake, nprobe 600 / reorder 1000 gives 0.9931 in 2,601 s, and nprobe 650 / reorder 1000 gives 0.9942 in 2,722 s (min after training 0.9859). The np650 config is submitted, pinned at `f21de886`. It is the highest recall at or below the 2,800 s pick line.
- **Killed on the way:**
  - E1 adaptive probing: neither the distance-ratio rule nor the radius-aware rule beats the fixed-nprobe curve.
  - PQ-25/20 with a larger reorder: worse recall at equal time.
  - AVX-512 LUT16 alone: -13%, below its gate. It is subsumed by the fused scan.

## 10. Balanced centres, retrain and probe schedule (2026-11-08, STATE.md agent S)

Goal: push the big-ann streaming entry (msturing-30M-clustered, final runbook, pq50 / reorder 1000) from mean recall@10 0.9931 toward 0.999 inside 2,800 s of summed step time on Ice Lake. All parameter choices below use held-out queries only (2,000 queries from MSTuring query100K that are not in `queries10k`, exact GT at the run10 states 3M / 6M / 10M, `~/bigann/l22bench/heldout/`); the board queries are used once, for the final confirmation. Held-out minus board recall was +0.0005..+0.0025 at np 400 / 600 (run18), so the held-out set is not harder than the board.

### 10.1 Where the recall loss is [M, held-out, np 600, reorder 1000, 3 states]
- Reorder is not the lever: reorder 5000 adds at most 0.0004 (run17).
- 66 of 2,000 queries miss anything, the worst query misses 2 of 10: no concentration on a few queries.
- A missed neighbour's best list sits at centre rank p50 755 / p90 1,173 for a probe depth of 600 (it is beyond the probe depth), in a list 2.7x the median size. The lists are very uneven: with the op32 centres, the 10M state has imbalance `lists * sum(len^2) / sum(len)^2` = 2.93, a largest list of 150,556 rows and a median of 2,244.
- The centres are trained once at 1.24M live rows (sample 262,144, 10 iterations) and the clustered insert order then moves the live distribution; the 3M-trained centres applied to 6M / 10M (K3) lose the whole balance gain.

### 10.2 Single-candidate kill tests (mean recall of the three states at equal scanned blocks, i.e. at the control's np 600 block count)
| candidate | recall | verdict |
|---|---|---|
| control (op32 centres) | .9943 | |
| k-means at the state, 10 it, 262k sample (K1) | equal recall per np, 37% fewer blocks | balance is the effect |
| 25 it, 262k sample (A1) | .9974 | |
| 10 it, 1M sample (A2) | .9979 | sample size matters, not iterations |
| 25 it, 1M sample (K2g) | .9980 | |
| 8,000 lists, 1M sample (B8k) | .9987 | place cost 2.4x, ~8-10% search gain: not adopted |
| SOAR lambda 0 / 0.5 / 2 | .9899 / equal / equal at np 600 | lambda 1 stays |
| selective spill (spill only within 1+eps of the primary distance), eps 0.5 | .9962 | killed: entries barely drop, recall falls |
| reorder 5000 | +0.0004 at most | killed (run17) |

### 10.3 Retrain (2x2 factorial, recall at equal blocks; stale = cold k-means at the 3M state applied at 6M / 10M)
| | 6M | 10M |
|---|---|---|
| control (op32 centres) | .9937 | .9966 |
| 262k sample, stale | .9953 | .9976 |
| 262k sample, 3 warm iterations | .9969 | .9989 |
| 1M sample, stale | .9960 | .9983 |
| 1M sample, 3 warm iterations | .9981 | .9991 |
Sample size adds +.0007 / +.0007, the warm retrain +.0016 / +.0013; the interaction (actual minus additive) is +.0005 / -.0005, i.e. additive within noise.

### 10.4 Rebalanced warm retrain, seed noise, and the first full run

Held-out static states (run10 op200/op1172/op648, 1M-sample k-means, reorder 1000), mean recall@10 over the three states:

| arm | np800 | np1000 | np1300 |
|---|---|---|---|
| balanced warm retrain (R1000) | 0.9973 | 0.9985 | 0.9994 |
| same, seed 8 | 0.9971 | 0.9983 | 0.9993 |
| same, seed 9 | 0.9967 | 0.9981 | 0.9991 |

Seed-to-seed spread is about 0.0003 at np800 and 0.0002 at np1000; differences below that are noise. Reorder 500/700/1500 stay within 0.0004 of reorder 1000. The k-means rebalance step (reseeding centres that own less than a quarter of the mean list onto members of oversized lists) gives the same recall as the plain warm retrain at equal nprobe (op1172 np1000: 0.9988 versus 0.9983; op648: 0.9996 versus 0.9994, both inside noise); its purpose is a lower post-retrain imbalance, not recall at fixed nprobe.

First full Ice Lake run (n2-custom-8-16384 clone, commit a19f4804, np1000, reorder 1000, 6 imbalance-triggered retrains): board mean recall 0.998617, post-train minimum 0.9978, peak RSS 6.5 GiB, but summed step time 3,750 s (insert 342, search 3,366, delete 42). Retrains cost 394 s in total (30.6, 73.5, 61.4, 100.5, 72.1, 56.1 s). The wall gate (2,800 s) fails by 950 s. Recall is 0.9986 at np1000, so the 0.999 target is within about 0.0004 of reachable by probing alone, but the time to do so is not available: the search term scales roughly linearly with nprobe (about 3.0 s per unit nprobe plus retrains).

### 10.5 Full-runbook result with the rebalanced retrain (E2a)

Ice Lake clone (n2-custom-8-16384), commit b15ec08e, np650, reorder 1000, six imbalance-triggered retrains with k-means rebalance:

| run | board recall | summed step s | insert / search / delete s | post-train min | RSS GiB |
|---|---|---|---|---|---|
| submitted control c5cd1c61, np650 | 0.994244 | 2,722 | | 0.9859 | |
| submitted control c5cd1c61, np600 | 0.993062 | 2,601 | | | |
| R1 (a19f4804), np1000 | 0.998617 | 3,750 | 342 / 3,366 / 42 | 0.9978 | 6.50 |
| E2a (b15ec08e), np650 | 0.996571 | 3,194 | 332 / 2,821 / 42 | 0.9943 | 6.51 |

Retrains in E2a: imbalance 5.0 to 1.74, 2.8 to 1.92, 3.8 to 1.98, 2.2 to 1.80, 2.3 to 1.85, 3.0 to 1.95, costing 31.7, 72.1, 60.7, 99.6, 70.5 and 55.7 s (390 s). The rebalance holds the imbalance after each retrain near 1.7 to 2.0 (R1 without it: 1.7 to 2.8).

Reading: at equal nprobe (650) the balanced configuration gains 0.0023 recall over the control, but the 390 s of retraining plus the extra per-step cost leave it 472 s above the control. The control's measured slope is about 2.4 s per nprobe unit; the balanced scan's is about 1.6 s per unit (R1 versus E2a). To fit 2,800 s the balanced configuration must give up roughly 390 s of search, which on that slope is about 250 nprobe units (np about 400), where static held-out recall (about 0.991 at np400) is below the control's 0.9942. Under the 2,800 s budget the control therefore stays the best measured point; the balanced configuration is better only if the retrain cost falls below roughly 100 s in total, or the budget exceeds about 3,200 s. 0.999 needs about np1000 plus retrains (3,750 s measured, recall 0.9986 even then), so it is not reachable in 2,800 s with these levers.

Miss audit (agent Y, LOWLEVEL-SYSTEMS.md): 93 to 95 percent of misses are IVF probe misses, so recall per scanned block, not the scan or reorder, is the quantity to move. The 50 busiest lists carry 18 to 25 percent of missed neighbours, consistent with the imbalance finding above.

E2b (identical configuration, second full run): board recall 0.996571 (bit-identical to E2a), summed step time 3,171 s versus 3,194 s (23 s, 0.7 percent run-to-run spread), search 2,801 s, same six retrains (31.3, 73.7, 61.1, 99.4, 70.0, 56.8 s). Recall is deterministic; time spread is under 1 percent, so the 400 s excess over 2,800 s is not noise. VM warm-bench-icelake-s was stopped after the runs (TERMINATED).

## 11. Generation 2: a cheap balanced retrain

Target: a full Ice Lake final-runbook run at or below 2,800 s summed step time with board recall above 0.994244 (the submitted c5cd1c61 at np650, 2,722 s). The balanced retrain of section 10 reaches 0.99657 at np650 but costs 3,194 s, of which 390 s is the six retrains. Per-retrain time splits into k-means on the warm sample (about 16 s on Ice Lake at 3 iterations over a 1M sample), and re-placing every live row (about 8 s per million live rows: a 5,000-centre GEMM and SOAR selection per row, plus encode and append).

### 11.1 Candidates and lineage

| id | change | parent |
|---|---|---|
| E2 | warm retrain, 3 iterations, 1M sample, exact placement (section 10.5) | b15ec08e |
| M1 | route each row through a super-centre: score only the `route_cands` list centres nearest its nearest of `route_lists` super-centres (k-means of the centres, 5 iterations); applies to inserts and to the retrain re-placement (`route_cands`, `route_lists`, `route_inserts`; 0 = exact, bit-identical to E2) | E2 |
| M2 | smaller warm k-means: 2 iterations over a 500k sample | M1 |
| M1' | rejected variant of M1: candidates from the new centres nearest the row's old primary centre (needs the old placement; recall 0.0003-0.0005 below exact at R512, same cost) | E2 |

Held-out screening (the run10 stage states op648 at 10M live and op1172 at 6M, 2,000 held-out queries, reorder 1000, replaying a warm retrain from centres trained at op200; retrain seconds measured on the wl-03 machine, which is about 1.7 times faster than Ice Lake):

| arm | state | retrain s (centres + place) | imbalance | recall np400 / np600 / np800 |
|---|---|---|---|---|
| exact, 3 it, 1M | 10M | 57.8 (9.0 + 48.2) | 1.61 | 0.9961 / 0.9983 / 0.9990 |
| M1' R64 | 10M | 21.5 (9.0 + 12.0) | 1.95 | 0.9902 / 0.9951 / 0.9977 reject |
| M1' R256 | 10M | 26.5 (9.1 + 16.9) | 1.79 | 0.9944 / 0.9974 / 0.9987 |
| M1' R512 | 10M | 29.9 (9.1 + 20.2) | 1.76 | 0.9956 / 0.9982 / 0.9990 |
| M1 S64 R256, 3 it, 1M | 10M | 24.9 (9.1 + 15.2) | 2.67 | 0.9933 / 0.9970 / 0.9983 |
| M1 S64 R512, 3 it, 1M | 10M | 27.9 (8.9 + 18.5) | 2.26 | 0.9951 / 0.9980 / 0.9989 |
| M1 S32 R512, 3 it, 1M | 10M | 28.0 (9.2 + 18.3) | 2.53 | 0.9949 / 0.9980 / 0.9988 |
| M1 S128 R256, 3 it, 1M | 10M | 25.3 (9.0 + 15.8) | 2.40 | 0.9928 / 0.9964 / 0.9981 |
| M1' R256, 1 it, 500k | 10M | 19.0 (1.7 + 16.8) | 2.07, max list 75k | 0.9921 / 0.9967 / 0.9987 reject |
| M1' R256, 2 it, 500k | 6M | 13.6 (3.1 + 10.1) | 2.18 | 0.9920 / 0.9965 / 0.9981 |
| exact, 2 it, 500k (M2) | 10M | 51.1 (3.1 + 47.4) | 1.69 | 0.9946 / 0.9981 / 0.9992 |
| M1 + M2 (S64 R512) | 10M | 22.1 (3.0 + 18.6) | 2.43 | 0.9944 / 0.9980 / 0.9992 |
| exact, 2 it, 500k (M2) | 6M | 31.8 (3.0 + 28.3) | 1.86 | 0.9919 / 0.9962 / 0.9982 |
| M1 + M2 (S64 R512) | 6M | 14.5 (3.0 + 11.1) | 2.83 | 0.9914 / 0.9963 / 0.9981 |

Readings: the warm sample can fall from 1M x 3 iterations to 500k x 2 with no recall change (3 s instead of 9 s of k-means); one iteration leaves one oversized list and loses recall. Routed placement matches exact placement within 0.0001 recall at np500 and above although the list imbalance it leaves is higher (2.4-2.8 against 1.7-1.9), and removes 60 percent of the retrain at 10M. Fewer super-centres or more candidates did not matter between S32 and S64 at R512; R256 loses 0.001 at np600.

Not tested: overlapping the retrain with inserts (candidate d). The 8-thread search and insert steps already use all cores of the 8-vCPU machine and the harness sums step times, so a background retrain would take the same cores and the saving would be an accounting effect rather than a speedup.

### 11.2 The metric that decides it: recall at equal blocks, not at equal nprobe

The screen above compares recall at equal nprobe. Scan time scales with the 32-code blocks scanned, and list imbalance changes blocks at a fixed nprobe, so the same screen read at equal blocks gives a different answer:

| 10M live, np600 | blocks | recall | imbalance |
|---|---|---|---|
| stale centres, no retrain (N0) | 212k | 0.9968 | 2.25 |
| exact warm retrain, 2 it / 500k (M2) | 200k | 0.9981 | 1.69 |
| routed placement, S64 R512 (M1 + M2) | 265k | 0.9980 | 2.43 |
| old-centre routed R512 (M1') | 235k | 0.9982 | 1.76 |
| local rebalance, hi=3 (L5) | 173k | 0.9967 | 1.31 |

Routed placement matches exact recall at equal nprobe but scans 19 to 33 percent more blocks, so at equal blocks it is about 0.003 behind. This is the reason it fails at full scale (11.3).

### 11.3 Full-runbook results of generation 2

Ice Lake clone (n2-custom-8-16384), board queries, reorder 1000, all steps summed:

| run | commit | change | board recall | summed step s | insert / search / delete s | retrains s | min post-train |
|---|---|---|---|---|---|---|---|
| submitted control | c5cd1c61 | np650 | 0.994244 | 2,722 | | | 0.9859 |
| E2a / E2b | b15ec08e | exact, 3 it / 1M, np650 | 0.996571 / 0.996571 | 3,194 / 3,171 | 332 / 2,821 / 42 | 390 | 0.9943 |
| S1 | c492ef1b | routed placement for inserts and retrains, 2 it / 500k, np600 | 0.996181 | 3,125 | 169 / 2,917 / 38 | 167 | 0.9927 |

S1 retrains at live 1.64M, 6.07M, 4.74M, 8.95M, 5.82M, 4.25M took 12.9, 31.0, 25.9, 42.9, 30.1, 23.8 s (centres 5.4 s each, tables 0.5 s, placement 6.8 to 36.8 s). Routing halves the insert time (332 s to 169 s) and cuts the retrains by 223 s, but search rises from 2,821 s to 2,917 s although np fell from 650 to 600 (about 79 s less expected): the less even lists add roughly 400 s of scan. Imbalance right after the retrains is 2.7 to 3.0 against 1.7 to 2.0 for exact placement. Routed placement is therefore rejected (insert routing alone was not run; its imbalance cost is unmeasured).

### 11.4 Candidate verdicts and the 2,800 s prediction

Per-candidate retrain seconds (Ice Lake, from the wl-03 screen times scaled by 1.7 and from the S1 log):

| candidate | retrain s at 10M live | screened result | verdict |
|---|---|---|---|
| E2 exact, 3 it / 1M | about 98 (E2a log: 99.4 at 8.95M) | reference | too slow |
| (b) 2 it / 500k warm sample | about 87 (centres 5 s instead of 15 s) | recall equal to 3 it / 1M, blocks equal | keep, saves about 60 s over the run |
| (b) 1 it / 500k | about 32 | one oversized list (75k), recall lower | reject |
| M1 routed placement (a variant of d: cheaper placement) | about 38 | +19 to 33 percent blocks | reject (S1) |
| (a) local rebalance, hi=3 (L5) | about 38 | imbalance 1.31, recall at equal blocks 0.0005 to 0.0008 below exact, 0.0017 above stale | closest |
| (a) local rebalance, hi=1.5 to 2 (L1 to L4) | 63 to 120 | re-places 6.5 to 12M rows, recall below exact | reject |
| (c) fewer retrains | not run | each retrain is roughly break-even in time-equivalent recall at 10M | not worth a run |
| (d) overlap with inserts | not run | cores are shared | accounting only |

Prediction from the measured components (inference, not measurement): holding recall at the E2a level, every variant above needs 330 to 390 s removed from E2a's 3,194 s; that is 210 to 250 nprobe units at the measured 1.58 s per unit, which lowers board recall by about 0.0042 at the static slope of 0.00002 per unit (0.9924 to 0.9927). The best case, L5 in place of the six exact retrains (saving about 270 s) at np about 570, predicts about 0.9944 at 2,800 s, which does not clear the 0.9945 kill threshold and is within the error of the control (0.994244).
