# Predictive equivalence after evidence withdrawal

The next scientific route is a compact predictive model whose behavior remains
equivalent to its surviving evidence. The output is an empirical outcome
distribution, rather than a packet containing every possible execution read.
This does not replace the temporal-fact or throughput qualification contracts.

The first prerequisite passes an exact local test: decide whether two
source-dependent histograms give the same normalized distribution after every
source withdrawal, without enumerating every withdrawal set. This is a mechanism
to investigate; the complete state-refinement follow-up appears below. No useful
agent, invention claim or competitive system result is established.

## Contract and derivation

Each observation has an outcome, a positive integer multiplicity, and an AND
dependency on at most `g` source identities. Withdrawing any dependency removes
that observation. Shared identities remain shared. No surviving observations
means `unknown`, distinct from every probability distribution. Observations are
supplied; no extraction or causal-identification claim is made.

Let `x_i` indicate source survival. For profile `s` and outcome `b`, define
`N_s,b(x) = sum(count_r * product(x_i for i in dependencies_r))`, and
`T_s = sum_b N_s,b`. Readout is `N_s,b/T_s` when `T_s > 0`. These polynomials have
degree at most `g`. Boolean products reduce with `x_i^2 = x_i`, implemented by
union of dependency sets.

Profiles are equivalent for all withdrawals exactly when both conditions hold:

1. Their nonempty-evidence conditions agree. Positive counts make each condition
   a monotone disjunction of dependency conjunctions. Removing conjunctions that
   contain another conjunction gives a unique minimal-set antichain; these must
   match.
2. For every outcome, the multilinear polynomial
   `N_s,b*T_t - N_t,b*T_s` has every coefficient zero.

Sufficiency follows by division when the shared support condition is true;
otherwise both return `unknown`. For necessity, the cross difference vanishes
where both totals are positive. Where either total is zero, its nonnegative
numerators are zero too, so the cross difference also vanishes. Uniqueness of
multilinear representation makes every coefficient zero. This uses established
Boolean polynomial mathematics, as described in
[O'Donnell's textbook](https://arxiv.org/abs/2105.10386).

Each cross difference has degree at most `2g`. A nonzero one has a distinguishing
withdrawal set of size at most `2g`: restrict variables one at a time, choosing
survival whenever that restriction remains a nonzero polynomial. If survival
makes it identically zero, choose withdrawal. Then the polynomial has factor
`(1-x_i)`, so remaining degree drops by at least one. The final nonzero constant
therefore requires at most `2g` withdrawals. The implementation returns a witness
and checks it against exact rational readouts.

The bound is sharp even with positive evidence and strictly positive predicted
outcomes after every withdrawal. [WITNESS-TIGHTNESS.md](WITNESS-TIGHTNESS.md)
constructs such a family and proves its first disagreement requires `2g`
withdrawals. All 11 declared families / 8,188 exact comparisons pass. Empty
dependencies are permitted in that construction. The subsequent
[matrix construction](MATRIX-WITNESS.md) attains the sharp `2g` bound with every
record depending on exactly `g` withdrawable sources, with no background evidence.
All five families / 37,448 comparisons pass; both outcomes remain positive whenever
the profiles are known, and both become unknown together. Thus neither positivity
nor the absence of unconditional evidence improves the general distribution bound.

**This bound does not cover unknown-state discrepancies.** One-outcome profiles
supported by `{0,1,2}` and `{0,1,3}` agree after every zero, one or two withdrawals.
Removing `{0,1,2}` makes only the first unknown. Their cross products are
identically equal. Ignoring support would incorrectly turn the bounded test into
a universal guarantee.

This handles arbitrary withdrawal counts only for the declared histogram
contract. Latest-record temporal selection adds products of blocking conditions,
so degree is not bounded by provenance width alone. Absolute counts, original
episode retrieval and arbitrary future inserts require additional information.
Empirical prediction equivalence does not establish causality or generalization.

## Frozen result

The [plan](../../../benchmarks/research/source_prediction/PLAN.json) and exact
prototype are frozen at
`/Volumes/A/.hms-bench-cache/warm-source-prediction-20261103-v1`.
Five regression tests pass. The experiment completed **8,597 profile pairs /
81,120 withdrawal comparisons** with zero candidate/oracle disagreements and
valid witnesses. Two exhaustive families contain 64 profiles and all 4,096
ordered pairs each; 400 six-source pairs also use every withdrawal set. Raw
profiles, rational readouts and pair results are saved.

Current-distribution-only merging was wrong on 1,166 pairs. Cross multiplication
without support checking was wrong on 622. A sound structural comparison that
normalizes source counts by their global gcd missed 27 valid equivalences.

Larger constructed cases multiply each outcome count by the number of surviving
sources. Predictions remain identical wherever evidence exists, although source
histograms differ. The 4/8/12-source cases were checked exhaustively; 16/20-source
cases each have 1,000 deterministic sampled checks in addition to the exact
polynomial identity. These are designed examples, not natural workloads. At 20
sources, the identity used 11,400 term products; no competitive speedup is claimed.

All four frozen source files and both raw-file hashes were verified. Freeze SHA256:
`63d5ff943de490d16ba0ac1f17efec5947a2475fd074f32b5b71c5476fcc0b3a`.
Receipts: `source-prediction-{freeze,finished}.json`; logs:
`source-prediction-tests.log`, `source-prediction-run.log`.

## Prior art and next comparison

[Action-conditioned bisimulation for agent memory](https://arxiv.org/abs/2609.38778)
already merges states by action outcomes and successor behavior.
[PRISM](https://www.prismmodelchecker.org/manual/RunningPRISM/ParametricModelChecking)
already supports rational parametric transitions and bisimulation minimization.
[Robust probabilistic bisimilarity](https://link.springer.com/chapter/10.1007/978-3-031-98679-6_12)
addresses sensitivity to small probability perturbations. Here correlated evidence
withdrawal also changes whether predictions are defined. That distinction
motivates investigation but does not establish priority.

The unresolved contribution is a useful source-withdrawal-aware predictive
abstraction: a common state quotient valid after retraction, with lower total
construction, repair or retained-state cost. Before promotion, compare with shared
**per-outcome** algebraic decision diagrams; terminals containing whole
probability vectors can create an artificial exponential disadvantage. Then test
complete action/state refinement, sequential withdrawals, rare transitions,
unstructured evidence and a real local behavioral workload. Charge state maps,
source dependencies, retained originals, scratch and repair work. This exact
screen supplies a basis for that work; it does not qualify milestones 4 or 6.

## Completed decision-diagram comparison

The independent control uses shared ordered algebraic decision diagrams with a
separate probability root for each outcome, shared exact rational terminals,
gcd normalization of recursive subproblems, and a special path that compiles only
the support condition for constant distributions. Both ascending and descending
source orders were fixed before the run. Four additional tests, including a
20-source constant-distribution case, bring the focused suite to nine passing
tests.

The frozen constructed cohort tests 4/8/12/16/20 sources with both uniform and
power-of-two weights, using three rotating repetitions. Eight cells completed
both diagram orders and produced exactly equal roots for the two histograms.
The polynomial method's median time was 0.0024–0.1230 of the faster diagram order
in those cells. The 16/20-source power-of-two cells reached the fixed 50,000
node/terminal limit in both orders; their ratios are explicitly absent, and
they are not counted as speedups. These short shared-host development timings
show a representation tradeoff on designed examples, not a system advantage or
an improvement over every symbolic algebra method.

Run: `/Volumes/A/.hms-bench-cache/warm-source-prediction-symbolic-20261103-v1`.
All eight source/protocol and ten raw-case hashes were verified. Freeze SHA256:
`9296145affaf957f7894120a578fec34534e895067db493fc97142a70f661c7d`.
Receipts and logs: `source-prediction-symbolic-{freeze,finished}.json`,
`source-prediction-symbolic-tests.log`, `source-prediction-symbolic-run.log`.
The following section adds complete state refinement; real behavioral evidence
remains open.

## Complete state refinement and repeated retraction

The next frozen screen implements a common labeled probabilistic bisimulation.
It starts with observable-label blocks and repeatedly separates states whose
action distributions into those blocks differ under any source withdrawal.
Unknown transitions remain observable. Every iteration only splits blocks, so
termination follows from the finite state set. Any uniformly valid partition
refines the initial labels and remains a refinement after each split; the stable
result is therefore the coarsest common valid partition. This is established
partition refinement with the source-equivalence predicate, not a new generic
bisimulation algorithm.

Each quotient block retains the observations of one canonical representative,
redirected to quotient targets, plus a map for every original state. All states
in a block have identical block-transition probabilities and unknown conditions
for every withdrawal. Induction preserves observable trace distributions for
any fixed action sequence. After withdrawal, these equivalences still hold, so
re-minimizing the retained representatives can discover the same additional
merges as rebuilding the common quotient from all surviving observations. This
preserves the predictive interface; it does not reconstruct individual episodes.

A stronger direct control is also included. When each observation has exactly
one source and no source-free contributions, unlimited-withdrawal equivalence
has a simpler characterization. Source support must match, and keeping only one
source forces its two conditional distributions to match. If all sources have
the same distribution, their mixture weights can differ freely. Otherwise,
keeping any pair of sources with different distributions forces their relative
weights to agree. The graph joining sources with different distributions is
connected, so all source weights must have one common proportionality factor.
Thus global normalized source histograms, with a special case for constant
distributions, completely decide the singleton-source case. Three added tests
include all 4,096 profile pairs against every withdrawal. The control also
simplifies constant distributions with joint provenance using their support
condition. This avoids treating ordinary single-source data as requiring the
more general algebraic method.

The full focused suite now has **17 passing tests**. The frozen state screen
completed **108 models, 16,908 transition checks and 1,074 sequential withdrawal
steps**. For 100 small random models, independent enumeration of all set
partitions and raw rational transitions found the same unique coarsest quotient.
Both decision-diagram orders agreed on all 108 models: 216 completed comparisons,
with no resource-limit omissions. Recompaction from the retained quotient matched
fresh minimization after every declared sequential withdrawal, including a case
where withdrawal enables a new merge. Current-only merging produced a different,
unsafe partition on four models.

The six deliberately correlated examples reduce four states to two and serialized
packets from 1,063–9,500 bytes to 275–780 bytes, including state maps. The diagram
control produces the same quotient and size. The independent-history stress case
retains all four states and remains 188 bytes: distinct source-support conditions
prevent merging even when current distributions agree. These are constructed
tests, not whole-process memory, token savings, natural workloads or a system
advantage. Their failure regime is especially relevant to real episodes, which
often cover different states.

Run: `/Volumes/A/.hms-bench-cache/warm-source-prediction-state-20261103-v1`.
All 14 source/protocol and 108 raw-model hashes were verified. Freeze SHA256:
`70960791eba091eb779f36ba1e0ac413d4ee8a840d52c1c9713f497409faa63d`.
Receipts and logs: `source-prediction-state-{freeze,finished}.json`,
`source-prediction-state-tests.log`, `source-prediction-state-run.log`.
The [first real-trace screen](PREDICTIVE-MINIWOB.md) now supplies 384 selected
human demonstrations. All observed state/action pairs have only one source;
withdrawal semantics preclude almost all active-state merging on both checkbox
tasks. Complete refinement and 48 sequential retractions preserve all 105,534
checked readouts. With dictionaries/maps included, the date packets save 6–9%,
while checkbox packets grow 2–4%. This exposed screen demonstrates no useful
general memory advantage; the exact semantic restriction is now a measured obstacle.
