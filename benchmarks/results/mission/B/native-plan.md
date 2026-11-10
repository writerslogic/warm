# H-B-6 native prerequisites

E24 validates the scalar specification according to the orchestrator. No native operation sequence is yet established at the required **10.71322046 billion row/query pairs per second**. Source semantics and the proposed machine sequence can be specified; sustainable throughput still needs measurement on a suitable free host. The target remains unchanged.

## Scoring boundary

`scoring-contract.json` records the pinned official scorer and its call chain. The score is a mean of ID-set recall against a stored float32 top-100 prefix, extended at rank10 using a strict1e-6 distance comparison. It does not recompute returned distances. Exact real L2, rounded BLAS L2 and this stored prefix are three distinct objects. Do not feed GT IDs, stored thresholds or known-wrong labels into serving. Keep official and independently audited scores and preserve disagreements.

The documented GT wrapper invokes an external DiskANN executable without recording its version. The [tagged DiskANN implementation](https://raw.githubusercontent.com/microsoft/DiskANN/0.6.1/apps/utils/compute_groundtruth.cpp), lines67-95 and195-215, uses float BLAS norms and norm-minus-dot distances with strict heap replacement. It illustrates rounding and tie-selection risk; it is not proven to have produced the published files. A numeric audit must establish omitted-neighbor correctness, not only recompute the distances of already stored GT rows. Freeze the scorer's NumPy version as well as input hashes.

## Exact refinement without an assumed float64 oracle

In the H-B-6 supported domain, every source coordinate is an integer multiple of2^-39: the lowest allowed binary32 exponent is-16 and the significand has24bits. Decode the bits directly into a signed integer. Its magnitude is at most2^55, a difference is at most2^56, its square at most2^112, and the sum over128dimensions at most2^119. Checked unsigned128 arithmetic therefore represents exact squared L2 in units2^-78. Compare these integers for candidate ranking and break exact ties by live tag. This does not establish parity with the official stored-GT tie policy.

An incumbent kth exact integer can be rounded upward to binary32 by bit/shift arithmetic; verify its enclosure against Fraction. This removes reliance on a rounded float distance to form U. Unsupported finite source coordinates or dimension129 use an exact fallback; they cannot enter a shorter packed kernel silently. This is a proof of a bounded representation, not a measured refinement cost. Python Fraction remains the independent fixture oracle.

The exact integer path must not be treated as free. The final runbook has640 x10000=6.4million query evaluations, not3.2million;3,213,966,138 is the sum of live-row counts at checkpoints. At4096seeds/query there are26,214,400,000seed distances. Applying E19's synthetic mean1020.6875additional rows/query gives6,532,400,000more distances, or32,746,800,000total100D distances; E20's mean243.875 gives1,560,800,000additional and27,775,200,000total. These extrapolations are work models, not official-data predictions. Scoring every E19 source distance in uint128 would require3.27468trillion coordinate evaluations; spending the entire600second reserve would require5.4578billion exact coordinate evaluations/second before updates, I/O and metadata generation. Integer decoding, full-width multiplication and carry accumulation all count. Even a passing dense-scan kernel would not establish this budget.

Use conservative incumbent upper bounds without first computing an exact score for every seed. A separately carded option below computes seed upper bounds from the same quantized representation. For admitted candidates, direct source-distance intervals can establish ordering when disjoint; exact uint128 fallback is needed only where intervals overlap the kth boundary or the final tie order is unresolved. A candidate is discarded only when its lower bound exceeds the kth actual-candidate upper bound. Keep enough candidate state to make that invariant true, then resolve ambiguous finalists exactly. Neither cheap source intervals nor a rare-fallback rate has yet been measured or proved within600seconds; both require operation accounting. The current all-exact H-B-5 counter provides correctness/count evidence, not an efficient native refinement implementation.

Quantization metadata also needs an actual native generation proof. The per-vector scale lies in[2^-23,2^10), and is a binary32 dyadic with denominator dividing2^46. Source values and reconstructed coordinates can be represented in units2^-46 with signed64 integers. Nearest/clipped codes have residual magnitude strictly below one scale: maxabs/rounded_scale<128, including the endpoint clip. Consequently each residual integer has magnitude<2^56, and the sum of squared residual integers over128dimensions is<2^119. Checked unsigned128 accumulation can therefore specify the exact error norm squared in units2^-92. Upward binary32 square-root selection can use bounded integer comparisons. Zero vectors are a separate exact zero-error case. The reconstructed norm is scale² times an integer code-square sum; represent its significand/exponent separately rather than forcing that whole value into the2^-92 accumulator. All these bounds require implementation checks and the smallest native correctness test below.

## Candidate ISA sequence

Pack16 live rows per block, in groups of four coordinates: `[block][coordinate_group][row_lane][four_bytes]`. Store data codes as unsigned `zx+128`. Broadcast four signed query bytes to each lane. At100D,25 ZMM `VPDPBUSD` instructions produce16 independent row dot products. No100-to128dimension padding is required. For other supported dimensions, zero-pad only the final four-coordinate group and mask inactive row lanes; metadata and dimension checks remain based on the original vectors. Dense live slots avoid scanning deleted rows.

With `A=sum((zx+128)*zq)`, compute `D=A-128*sum(zq)` in signed32. For dimension<=128, `2D` is exactly convertible to binary32. The scale values are positive. This permits the following sequence, where RD/RU mean correctly rounded binary32 operations toward negative/positive infinity:

```
df = exact_float32(2 * D)
t  = RU(sq * df)
p  = RU(sx * t)
n  = RD(nx_lower + nq_lower)
L  = RD(n - p)
Bq = RU(sqrt_RU(U_upper) + eq_upper)       [once per query/incumbent update]
r  = RU(Bq + ex_upper)
R  = RU(r * r)
prune = valid_lane && (L > R)
```

`p` is an upper enclosure even when D is negative: multiply the first upper enclosure by positive sx. This avoids computing both endpoints of sx*sq and selecting by dot sign. It changes the E24 operation order and may change false keeps, so native checks must establish enclosure against the independent oracle; E24 bit equality is not presumed. Clamping L to zero is unnecessary for the pruning decision because R is nonnegative. No FMA/reassociation is assumed. Validate scales, radii, norms and thresholds before the inner scan; exceptional lanes refine safely. Establish and restore each worker's floating environment; embedded rounding alone does not validate FTZ/DAZ behavior.

This entails six vector floating arithmetic operations per16pairs, plus integer correction/conversion, validity/comparison/mask handling, loads and loop work, after25 VNNI instructions. Intel's pinned `intel-directed-rounding.json` defines the directed add/sub/mul/sqrt primitives; `intel-vpdpbusd-performance.json` gives Icelake Xeon VPDPBUSD latency5 and reciprocal throughput0.5cycles. At four cores and nominal2.8GHz, the whole kernel has only1.04544core-cycles/pair, while dots alone consume at least0.78125 under that isolated-instruction ceiling. The remaining0.26419cycles/pair is not a measured epilogue budget. Port sharing, issue bandwidth, sustained frequency, metadata loads and candidate extraction may defeat it. At least ten independent dot chains are necessary merely to cover the stated latency; a fixed8queries x32rows tile gives16 ZMM accumulators before other live registers. That tile is a proposal, not a selected performance result. Data reuse across query tiles and total loads must be counted.

Existing `src/core/qgraph/kernels.rs:702-795` supplies signed-query integer-dot dispatch, but the inspected helpers require dimensions divisible by16 and return horizontal row dots. They do not implement this16-row packed full scan or its certificate epilogue. Existing partition search already reuses PQ work across query tiles; do not credit batching itself as a new mechanism. Reuse its dispatch/error conventions where applicable without pretending the present helper executes K100 packing.

## Ownership and memory

This design replaces `PartitionTier`'s trained IVF/PQ representation. It does not run the existing2865-second workload and then append a full dense scan. `src/core/qgraph/partition.rs:178-193` currently owns raw rows, slot IDs, ID-to-slot HashMap, free slots and `Trained`. `Trained` at138-151 owns centers, Gram matrix, codebook, per-list tables, duplicated memberships and location arrays. Training at995-1104 additionally gathers live IDs and training samples. The new path must not retain these old structures, training samples or a second full raw copy.

Use one bounded live arena and its packed-code arena, plus scale/norm-lower/error-upper metadata and live tags. Reserve a checked maximum live capacity at setup, or use bounded pages; avoid Vec doubling of a multi-gigabyte array. Keep a compact live prefix: deletion swap-moves the last live row/code/metadata/tag into the hole and updates that tag's map entry. Replacement validates all inputs first, then overwrites that tag's row and metadata without increasing live count. The API serializes mutation and search; a future concurrent API would require a separate snapshot/reclamation design. All batch validation must precede mutation or provide rollback. The present `PartitionTier::insert` checks duplicate IDs while mutating, so its failure atomicity cannot simply be assumed from the binding's comment.

Recompute the fixed H-B-5 data-only seed set from current live tags at each checkpoint; do not cache query answers between checkpoints. A bitset prevents duplicate seed/refinement admission. Only transient per-query candidate state is retained. `StreamingRunner` holds every returned result array through completion: returning views into a reused native buffer would corrupt earlier checkpoints. Keep distinct result storage as required by that caller.

The following is a **planning budget**, not a measured peak. C=10,292,043 comes from final-runbook metadata and the existing range-count replay; an independent live-tag replay must still verify it. The largest source insert range is757,735rows from the pinned runbook. All byte counts below are decimal bytes.

| Owner | Planned bytes at maximum |
|---|---:|
| One live raw100D binary32 arena | 4,116,817,200 |
| One packed100-byte code arena | 1,029,204,300 |
| Three binary32 metadata fields | 123,504,516 |
| Live slot tags | 41,168,172 |
| ID map allocation budget | 268,435,456 |
| Harness640 separate int64 result matrices | 512,000,000 |
| Query matrix | 4,000,000 |
| Seed bitset and copied4096rows | 2,924,906 |
| Largest input batch plus uint32/int64 IDs | 312,186,820 |
| Packed tiles and worker scratch allowance | 8,388,608 |
| Interpreter, libraries, stacks, allocator/HDF5 allowance | 268,435,456 |
| Reclaimable file-cache allowance | 1,073,741,824 |
| **Total planning budget** | **7,760,807,258** |

This leaves829,127,334bytes below8GiB before any unmodeled use. The ID-map budget is deliberately larger than the approximate151MB bucket/control calculation for2^24eight-byte entries plus control bytes; the actual HashMap layout, capacity, tombstone cleanup and rehash peaks must be measured. The page-cache allowance is not an enforced independent limit. Cgroup memory includes charged file cache and can exceed process RSS; reclaimability and insertion/replacement high-water marks need a full lifecycle receipt. Small packing alignment tails are charged to scratch. Do not allocate the entire source, a full pairwise score matrix, all GT matrices, or duplicate full-size index arenas inside the serving process.

The range-reader adapter at `/Volumes/A/.hms-lb/bigann/adapter-l22/new/neurips23/streaming/warm/streaming_io.py:10` validates headers and reads only requested rows. Preserve that ownership boundary. The Python partition binding at `src/bindings/python/partition_index.rs:47-67` borrows contiguous input or allocates a contiguous copy; the adapter's contiguous float32 conversion normally permits borrowing, but a general noncontiguous caller can add a whole batch copy and must be budgeted or rejected by an explicit API contract. Retained result arrays are part of the harness budget even though they are outside the index object.

The prior receipt `benchmarks/results/azure/az1-2026-11-09/streaming-e2a-1/summary.json:11` gives6,846,004KiB=7,010,308,096bytes=6.528858GiB process peak. The receipt is on E8bds_v5, not official D8lds_v5, and is not a cgroup peak. Its component capacities and allocation timestamps were not captured. Known old categories include raw rows, approximately two25-byte PQ codes and two4-byte list slots per live row,16-byte locations, a100MB5000x5000Gram matrix,16MB list tables, training scratch and accumulated results. Their peaks need not coincide; subtracting their nominal sizes from that RSS is invalid. The new total above is therefore a fresh design budget, not a reconciliation claiming memory savings. A measured ownership census plus cgroup `memory.peak` across all operations is required.

## Smallest next action: H-B-6N native arithmetic gate

Limitation: E24 tested exact-rational software endpoints; native scale/error generation, uint128 refinement and the reordered epilogue are untested. Prediction: the fixed native contract preserves all Fraction enclosures and exact candidate ordering. This is a numerical prerequisite, not a new search algorithm or a tuning sweep.

Implement a standalone native fixture target in a newly authorized owned file; no engine change. It should implement the reordered epilogue, exact fixed-grid refinement, checked metadata construction and explicit fallback. Compare independently with Fraction on the fixed E24 boundary classes and dimensions99/100/101/127/128/129, plus uint128 coordinate/difference/sum maxima and scale endpoints. Include the caller-composition invariant: kth threshold formation from actual candidate distances, exact ties retained, and no false pruning. Exercise both initially nondefault rounding states and restore the caller's state on success and error. Any unsupported source must report fallback; no environment-dependent silent acceptance.

On the verified free local host, a strict scalar C backend with standard floating-environment access can validate native directed arithmetic and uint128 on arm64. [Clang documents strict floating behavior and environment access](https://clang.llvm.org/docs/UsersManual.html#controlling-floating-point-behavior). Use `#pragma STDC FENV_ACCESS ON`, `-ffp-model=strict -ffp-contract=off`, inspect generated instructions and retain compiler identity. That backend's per-operation environment handling is a correctness vehicle, not an epilogue throughput proposal. Acceptance: zero enclosure, exact-distance, ordering, fallback or environment-restoration failures. Cap180seconds and record failure on timeout; no parameter search. E24 itself need not be repeated unchanged.

The local read-only capability receipt on2026-11-10 is arm64 Apple M4,34,359,738,368bytes RAM,10physical/logical cores; `hw.optional.arm.FEAT_DotProd=1`, `hw.optional.neon=1`; Apple clang21.0.0, targetarm64-apple-darwin27.0.0. Existing signed NEON dot paths are available for a separate local packed-dot correctness exercise. This host cannot execute AVX-512/VNNI. No other currently available free x86 host was verified in this brief, and no paid machine was started.

After native arithmetic passes, the smallest relevant throughput action is one fixed VNNI16-row x8-query/32-row tile implementation on a verified free Ice Lake server host, with CPUID/OS-state gating and code inspection. Use100D, a1M-row data-only code/metadata arena,256 fresh generated queries, one untimed warmup and one recorded run; cap60seconds, consume mask/count checksums and include epilogue and candidate extraction. Count every processed pair; do not time only dot products or replay cached masks. Report the run, observed frequency, process/cgroup memory and compiler flags. A large enough arena must exceed measured LLC; otherwise the result is only a cache-resident kernel result. Require at least10.71322046Gpairs/s in the recorded fused run to advance, while recognizing that this is necessary, not sufficient, for full8GiB/3600s serving. If no eligible free host is available, continue scalar native correctness, packing proof and lifecycle implementation; do not spend on a diagnostic.

The next code files are not part of this brief, so no nonexistent build/test command is represented as ready. Root should assign that small native target, then execute its concrete command once its artifacts are reviewable. Existing unchanged E24 artifacts remain the scalar oracle. No build, test, benchmark, install or paid call was run for this audit.

## Separate smallest count prerequisite: H-B-6U interval incumbents

Limitation:4096exact source seed distances/query may consume the600second reserve. Mechanism: keep the same deterministic4096live-tag seed selection and existing int8 quantization. Enclose reconstructed squared distance from above using stored reconstructed norm upper bounds and the scaled-dot lower endpoint. Set each seed's actual-distance upper bound to `RU((sqrt_RU(Dhat_upper)+ex_upper+eq_upper)^2)`, and use the kth smallest such bound. The triangle inequality makes it an actual-candidate upper bound without reading the raw seed vectors per query. It does not certify seed ranking or permit approximate final results.

This needs one additional binary32 norm-upper field per live row,41,168,172bytes at the maximum, bringing the planning total to7,801,975,430bytes before any additional candidate interval state. Seed byte dots cost26.2144billion pairs across the stream, about0.0816% of the required dense pairs, plus their upper epilogues and4096square roots/query. That last term is26.2144billion scalar-equivalent square roots and must be measured or bounded; it is not free. A monotone squared comparison may avoid some roots in a later fixed selection implementation, but that is not assumed here.

Prediction: the representation-derived upper thresholds retain the H-B-5 mean<=2048 and samplep99<=8192additional-refinement gates on both existing synthetic distributions. Lowest rung: one unchanged first16-query development prefix with the new fixed seed-upper formula, same4096-row refinement blocks and exact source refinement for survivors initially. Compare every pruning decision with the independent oracle, record seed threshold inflation and candidate counts, and never use the oracle in decisions. The second distribution follows only after the first passes. Cap600seconds, no seed-size/code-width/order sweeps. Reject this upper-seed variant on any false prune or count-gate failure; do not retire exact seeding merely because this upper bound is loose.

Only after that gate should a separate fixed source-interval refinement implementation be judged: record intervals computed, uint128 fallbacks, ambiguous-finalist counts, bytes loaded and full seed/refinement time. Its cost must fit the reserved allowance together with updates, source I/O and metadata construction. The H-B-6N arithmetic target is still the immediate action and can proceed without this optional H-B-6U experiment. No new counter or native implementation was added by this brief.
