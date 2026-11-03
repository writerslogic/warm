# Bounding the score that the engine actually computes

The dense tree currently rounds each endpoint product and partial sum upward.
That supplies a conservative real-arithmetic interval, but the search contract
uses a particular sequential floating-point dot product. For that contract,
ordinary correctly rounded operations already preserve the required ordering.
This is an application of standard floating-point monotonicity, not a claim to
have invented interval arithmetic or branch-and-bound search.

For each coordinate choose `e_i = high_i` when `q_i >= 0`, and `low_i` otherwise.
Every stored row `v` satisfies `low_i <= v_i <= high_i`. Hence the exact real
products satisfy `q_i*v_i <= q_i*e_i`. Rounding to the same floating-point format
is monotone, so their separately rounded products obey the same inequality.

Initialize both accumulators with the same negative-zero identity and add terms
in the same coordinate order. If the row accumulator is no greater than the
endpoint accumulator before a step, adding their respective ordered products and
rounding again preserves that inequality. Induction therefore bounds the entire
**computed** sequential dot product. Clamping both results to `[-1,1]` also
preserves order. Equality must still retain the node for score ties.

The implementation validates finite `f32` endpoints, finite query coordinates of
magnitude at most one, and at most 4,096 dimensions. Products and partial sums
cannot overflow `f64` under these bounds. Subnormal rounding, cancellation and
signed zeros do not invalidate ordinary numerical order. No real-dot-product
error guarantee follows: the endpoint result can itself round below the exact
real sum. For example, a point box with coordinates `(1, smallest-positive-f32,
-1)` and equal positive query coordinates loses its middle contribution during
sequential addition; its computed score is zero while the real sum is positive.
The bound must cover the computed zero. The proof also requires the same reduction order and arithmetic;
reassociation, an independently fused implementation or another score contract
would require a new argument.

The pinned Rust 1.96.0 source implements floating-point `Sum` as a left fold from
`-0.0`, matching `Embedding::cosine`. See the
[Rust implementation](https://doc.rust-lang.org/src/core/iter/traits/accum.rs.html)
and [Goldberg's account of correctly rounded arithmetic](https://docs.oracle.com/cd/E19957-01/806-3568/ncg_goldberg.html).
The local toolchain source was inspected as well; current online source is not
used to silently substitute a different compiler behavior.

The proposed implementation removes two `next_up` operations per coordinate
while retaining coordinate selection, sequential accumulation and clamping.
Its mathematical contract is narrower than a bound on ideal real arithmetic.
The experiment must check all existing adversarial dense-tree tests, additional
point-box and dimension boundaries, complete result equality on exposed corpora,
useful low-dimensional pruning and full query timing. No production change or
performance pass is established by this derivation alone.

## Frozen development screen

All 156 native document tests pass, including the additional point-box checks
at 14 dimension boundaries through 4,096. Two ignored cases are explicit timing
benchmarks. The first preflight required qualification of a relocated control's
module path; the second exposed a zero-vector setup in the existing dimension-one
fixture. Both failures are retained, and only the control path and new fixture
were corrected. Existing numerical checks were not weakened.

The frozen three-way comparison uses the preserved product, the earlier
materialization-only candidate, and the combined candidate. It declares 36 cells,
3,456 query rows and 6,912 paired full-result comparisons, including four
low-dimensional pruning controls. All nonempty filtered corpus cells must improve
their median by at least 10%; remaining controls may not regress more than 15%.
Every analytic query must complete the bounded tree with fewer dots than a scan.

Root: `/Volumes/A/.hms-bench-cache/warm-monotone-bound-20261103-v1`;
freeze `d7c3e01084550269eba4f4dfecaf5394f947ea64bd4f060a2407a082092ec4f0`;
461 source files, the pinned Rust `Sum` source, original compiler failures and
four exposed input hashes are retained. Launcher PID 47928 started at 16:13:13Z.
The screen finishes at 16:15:01Z with all 6,912 complete-result comparisons and
all analytic pruning checks passing. The performance gate fails in four cells:
trained 10% filters improve only 1.084–1.091x on COCO and 1.059–1.060x on DEEP.
Trained 1% filters improve 1.25–1.26x and 1.12–1.14x, respectively. Untrained
filtered cells improve 1.28–2.03x; those gains include the materialization change.
The separate intermediate control identifies each component's measured effect.
All other timing controls pass. No product integration follows this failed gate.

All 461 sources, frozen artifacts and outputs were hash-verified. Receipts:
`monotone-bound-v1-{freeze,launched,finished,summary}.json` and preflight log.
