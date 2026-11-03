# Constructing small certificates by following their proofs

The algebraic source lift establishes feasibility, but its first size screen
failed. The next experiment tests exact semantic pruning after that lift. A
subsequent stronger-control audit prevents its early speedup from being mistaken
for a research advantage. Both experiments use local computation and supplied
symbolic equations; neither establishes natural-language memory utility.

## A proof-directed minimizer

For a retained row `v` with provenance `T`, find a linear proof of `v` from the
other retained rows. A withdrawal that makes `v` essential must destroy at least
one row in this proof. It must also leave every source in `T` available; otherwise
`v` itself disappears and cannot witness a changed span.

Branch only on the sources in the current proof, excluding `T`, and recurse to
the declared withdrawal budget. If no proof exists at a node, its withdrawn
sources are an explicit counterexample to removing `v`. If every branch retains
a proof, `v` can be discarded. Memoizing visited source sets avoids repeated
branches. Testing rows in reverse identity order produces an inclusion-minimal
certificate in one pass: a previously essential row cannot become redundant when
more alternatives are removed. This does not guarantee globally minimum size.

Every retained row has a saved essentiality witness, checked against the final
certificate by a separate row reducer. Regression tests independently enumerate
withdrawal sets and actual finite-field solutions, including rare information,
joint provenance and contradictions. They also verify proof coefficients, witness
tampering, composition-related invariants and beyond-budget refusal. Five tests
passed; see `source-lift-refinement-tests.log`.

The frozen [refinement plan](../../../benchmarks/research/source_lift_refinement/PLAN.json)
uses new seeds 4101–4103, 64/256 sources, eight rows per source, and single/joint
provenance: 12 cells with three rotated repetitions. It compares the lift followed
by pruning with per-provenance compression, optimized exhaustive union, union
followed by pruning, and direct pruning without the lift. Union construction
compresses each provenance group first and stops once it reaches the original
rank; it does not retain the avoidable full scans of the preceding experiment.
All admissible withdrawals are verified. Cached checks reuse only exactly
identical surviving source subsets; the per-provenance control is verified
groupwise, which implies all-withdrawal equivalence by union.

Raw run: `/Volumes/A/.hms-bench-cache/warm-source-lift-refinement-20261103-v1`.
The complete experiment preserved the exact span in all 12 cases / 209,868
withdrawal states for every completed compiler. The source and plan were copied
and hashed before execution, and all raw-file hashes were verified afterwards.
Freeze SHA256:
`939eb10187d86dfb9f95ce73caf738f72b37755c8ac8561869f83a329f88ad9d`.
Each compiler has a 30-second deadline, including its separate allocation-tracing
invocation; timeout or missing comparators reject the screen. The log is
`source-lift-refinement-run.log`; receipt copies are
`source-lift-refinement-{freeze,finished}.json`.

The first single-source cells passed their development screen. One 256-source
case produced the same 850-byte packet as its three compact controls, with an
approximately 13x construction-time ratio against the fastest of those controls.
That was a partial, shared-host development measurement. All six joint-source
cases exceeded the direct-pruning allocation deadline, and the lift also missed
the construction-ratio condition in that family. The required all-cell screen
**failed**. The stronger-control result below supersedes the apparent advantage
even in the single-source cells.

## A stronger control: recurse only when a basis is hit

While reviewing that apparent gain, a simpler exact construction emerged. Given
the current withdrawn set `D`, compute the ordered greedy basis `B(D)` of the
remaining evidence. Add its original rows to the certificate. Recurse only on
additional sources appearing in `B(D)`, up to budget `k`.

Why this is sufficient: deleting sources outside a greedy basis leaves that
basis unchanged. Its selected rows still appear in the same order and span every
other surviving row. For any admissible final withdrawal set `F`, either the
current basis survives `F`, or some source in both `F` and the current basis leads
to one of the recursive branches. Following those branches eventually reaches
the same basis that exhaustive enumeration would produce for `F`. Every visited
withdrawal is itself admissible. Consequently, the recursive union is exactly
the exhaustive greedy union, not just a certificate of equal rank.

With rank at most `r` and at most `g` sources per row, each node has at most `gr`
branches. The search visits at most `sum((gr)^i, i=0..k)` nodes before memoization,
independent of the total source count. Rank loss can still require scanning all
remaining evidence at a node; that cost is retained. This is a bounded-withdrawal
decision procedure, not an unlimited-forgetting guarantee.

[basis_control.py](../../../benchmarks/research/source_lift_refinement/basis_control.py)
implements this construction, with optional use of the same minimizer. Two tests
passed: 120 random single/joint-provenance cases matched the exhaustive union
exactly, and rank-loss/contradiction examples preserved all changed conclusions.
The existing 256-source/seed-4101 case required only six basis calculations and
130 examined evidence rows after per-provenance compression, instead of 32,897
withdrawal combinations. Its packet was byte-identical to the 850-byte exhaustive
union. One diagnostic timing was about 0.0255 seconds versus the earlier lift's
0.1203-second median; those unmatched timing samples are not a qualified ratio.
See `source-basis-control-diagnostic.json` and `source-basis-control-tests.log`.

A separate frozen audit completed at
`/Volumes/A/.hms-bench-cache/warm-source-basis-control-20261103-v1`. It repeated the
lift and both new controls in three rotated repetitions on the same exposed
development inputs. Every new-control packet was byte-identical to its already
exhaustively verified union counterpart; the 209,868-state correctness evidence
therefore transfers without repeating the same audit. Source and raw-file hashes
were verified. Freeze SHA256:
`c0f16a8755b63364c344c803b1915209e0cb45e930bb514aacb7a80c455cc97f`.
Logs and receipts are `source-basis-control-run.log` and
`source-basis-control-{freeze,finished}.json`.

The supposed lift advantage disappeared in **all 12 cases**. At matched packet
size, its construction-time ratios against the stronger control ranged from
0.0177 to 0.1868, all below one. These remain shared-host development timings,
not qualified system speedups. The deterministic work reduction is more direct:
the new oracle used 6–116 basis calculations across the cohort, compared with
2,081 or 32,897 exhaustive withdrawal combinations. It charged input compression,
rank computation and evidence scanning; the six-call diagnostic did not measure
only the recursive part. No original-source data, calibration inputs, thresholds
or failed cells were replaced to obtain this result.

## Scientific interpretation

The useful lead is proof-guided memory: reason about the few sources supporting
a conclusion, then explore only withdrawals capable of invalidating those proofs.
It changes what memory is asked to compute; nearest-neighbor emulation supplies
no such guarantee. The algebraic construction supplies a valid bounded
intermediate representation, but the simpler proof oracle may make that
intermediate representation unnecessary for these workloads.

These ingredients connect to established representative families, kernelization,
hitting-set branching and bounded-deletion learning; see the primary sources in
[SOURCE-LIFT.md](SOURCE-LIFT.md), especially
[Cherapanamjeri et al.](https://arxiv.org/abs/2506.13048).
No novelty claim follows from deriving or implementing them here. A useful
research advance still requires a distinct practical tradeoff with source
extraction, temporal facts, contradictions and real reader workloads. The
original throughput preselection and all milestone gates remain unchanged.
