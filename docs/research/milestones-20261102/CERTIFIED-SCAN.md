# Certified arithmetic screen

The owner-membership screen still misses its performance gate. A separate
candidate changes the reduction dependency chain, while treating the existing
sequential `Embedding::cosine` as the authoritative computed score. It supplies
intervals for exclusion and uses that original scorer for every surviving row.
The frozen screen preserves every result but fails its performance rule. There
is no production change.

Let `d` be the dimension, `M` the largest absolute stored `f32` coordinate,
`u = 2^-53`, and `S = sum(abs(q_i*v_i)) <= d*M`, since a normalized query has
`abs(q_i) <= 1`. Four independent accumulators and their final reduction have
at most `d+3` rounded additions on a product-to-result path; the original
sequential sum has at most `d`. Products are evaluated in `f64` after exact
widening of the stored values. For `1 <= d <= 4096`, finite inputs cannot
overflow `f64`, even for the largest finite legacy `f32` coordinate.

The standard `gamma_n = n*u/(1-n*u)` analysis bounds each evaluation's error
relative to the real dot. Adding the two errors bounds their difference by
`(gamma_(d+4) + gamma_(d+1))*S`, plus the absolute underflow terms. The deliberately
looser radius `R = 8*d*d*f64::EPSILON*M = 16*d*d*u*M` dominates this bound throughout
the declared dimension range. If `M>0`, it is at least `2^-198`; even `O(d)`
smallest-subnormal `f64` rounding errors are far smaller. If `M=0`, both dots are
numerically zero. Signed-zero ties are retained for canonical re-scoring.

`d*d` has at most 25 significant binary digits and `M` at most 24; their product
fits `f64` exactly, and multiplication by the power-of-two scale remains normal
and finite. The implementation nevertheless rounds the final subtraction down
and addition up with `next_down`/`next_up`. Clamping both endpoints to `[-1,1]`
preserves containment of the clamped canonical score. This is a bound on two
computed evaluations, not permission to replace reported scores with estimates.

The kth-largest lower endpoint is no greater than the true kth-largest canonical
score. A row can therefore be excluded only if its upper endpoint is strictly
below that threshold or the requested minimum score. Equality stays eligible;
the unchanged scorer and `(document id, chunk)` ordering resolve all ties.
Candidate-limit selection and fusion remain unchanged. Invalid dimensions or
nonfinite estimates return `[-1,1]`, causing conservative canonical evaluation.

The error analysis and parallel reduction are established numerical methods:
[Higham, 1993](https://nhigham.com/wp-content/uploads/2023/10/high93s.pdf), and
[Jeannerod and Rump, 2013](https://epubs.siam.org/doi/abs/10.1137/120894488).
The latter's sharper no-underflow theorem is not required by the conservative
radius above. Selective exact evaluation is also an established pattern; no
novelty claim follows from this candidate.

The frozen decision rule will require 1.25× speedup in every nonempty filtered
development cell, unchanged full results, and all existing control/pruning gates.
Plan: `benchmarks/research/filtered_dense/CERTIFIED-SCAN-PLAN.json`.
Compilation and timing follow the completed Android qualification. No held-out
input or paid resource is involved.

An independent exact-rational check verifies the conservative inequality for
all 4,096 permitted dimensions, including an absolute underflow allowance;
the smallest radius-to-allowance ratio is approximately 2.2857 at dimension 1.
It also checks exact representability of 73,728 selected radius products across
the `f32` exponent range. This audits the stated algebra, not the native kernel.
Receipt: `certified-radius-check.json`; reproducible script:
`benchmarks/research/filtered_dense/check_certified_radius.py`.

The isolated 469-file preparation is
`/Volumes/A/.hms-bench-cache/warm-certified-scan-20261103-v1`; it retains both
earlier controls and includes the current platform-locking fix in their shared
core. All 159 document checks passed, including interval containment and near-tie/
filter-intersection regressions; two declared benchmark cases were ignored by
the preflight. Runner: `mac-certified-scan.py`. The timed screen started at
2026-11-03T17:31:33Z, PID 53527, after freezing all source, inputs, runner and
preflight artifacts. Freeze SHA256:
`90661689556f8d689adc057e6a97d2fa7531dd6f46a2b0c4f41199800ccce147`.
Receipts: `certified-scan-v1-{freeze,preflight,launched}`.

The screen completed at 2026-11-03T17:33:15Z. All 36 cells, 3,456 query rows and
6,912 paired full-result comparisons passed correctness; all control/pruning
gates passed. Six trained filtered cells failed the fixed 1.25× performance gate:
COCO 10% baseline/churn reached 1.238×/1.244×; DEEP 10% reached 1.024×/1.021× and
DEEP 1% reached 1.086×/1.087×. The other filtered cells reached 1.399×–1.735×.
Only 100 of each 10,000 or 1,000 scanned rows normally needed canonical scoring,
so successful arithmetic exclusion did not yield the required full-query gain.
The DEEP trained cells also regress against the preceding owner-membership
candidate. This rejects the combined implementation under its declared rule;
it does not invalidate the containment argument or establish a competitive gain.
No rerun or threshold adjustment is used to turn the failure into a pass.

All 469 source files, inputs, artifacts and outputs were reverified. Receipts:
`certified-scan-v1-{finished,summary}.json`. Raw query SHA256:
`e443803636cc724912012b989302bc4fd307fb9a05df218018201305b7e39ae8`.
