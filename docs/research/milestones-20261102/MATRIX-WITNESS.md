# The deletion bound is sharp without background evidence

The `2g` distribution-witness bound remains sharp when **every observation has
nonempty, withdrawable dependencies**. Positive outcome counts do not lower it.
The following construction improves the earlier anchored `2g-2` lower bound in
[WITNESS-TIGHTNESS.md](WITNESS-TIGHTNESS.md), without changing the prediction or
unknown-state contract. This is a mathematical constraint, not a system advantage.

## A three-source example

Two profiles have the following positive integer outcome counts for each source:

| Source | Left outcome 0 | Left outcome 1 | Right outcome 0 | Right outcome 1 |
|---|---:|---:|---:|---:|
| a | 4 | 16 | 1 | 19 |
| b | 10 | 10 | 20 | 20 |
| c | 16 | 4 | 19 | 1 |

With all sources present, both normalized predictions are `(1/2,1/2)`. Removing
only `a`, `b`, or `c` leaves the same first-outcome probability on both sides:
`13/20`, `1/2`, or `7/20`, respectively. But removing `b` and `c` leaves left
probability `1/5` and right probability `1/20`. Both outcomes are still positive.
Removing all three makes both profiles unknown. There is no background record.

Let `a,b,c` also denote Boolean survival variables and arrange each profile as a
row of a matrix:

```text
M(a,b,c) = [ 4a+10b+16c   16a+10b+4c ]
           [  a+20b+19c   19a+20b+ c ]
```

Using `a²=a` and `c²=c`, its determinant is
`60(a-c)(1-b)`. It is nonzero exactly when `b` is withdrawn and exactly one of
`a,c` survives: two withdrawals are necessary and sufficient.

## Compose the obstruction to any width

For an integer `g >= 1`, take `g` disjoint three-source groups and form
`P = M₁ M₂ ... M_g`. Interpret the two rows of `P` as binary outcome-count
profiles. Expanding a matrix entry sums products along intermediate two-state
paths. Every resulting observation has a positive integer multiplicity and
depends on exactly one source from each group: **exactly `g` dependencies**, all
nonempty. No negative evidence or permanently retained observation is introduced.

If every group has at least one surviving source, each matrix has strictly
positive entries, as does their product. Both profiles therefore have positive
counts for both outcomes. If a group loses all its sources, its matrix is zero
and both profiles become unknown together. Their unknown-state conditions agree
for every withdrawal set.

Two known binary profiles have equal normalized distributions exactly when their
count matrix has determinant zero. Determinants multiply, so

```text
det(P) = 60^g * product((a_i-c_i)(1-b_i), i=1..g).
```

Disagreement requires each group to retain only `a_i` or only `c_i`. Each group
then loses exactly two sources, giving **exactly `2g` withdrawals**. Fewer cannot
distinguish the profiles. This attains the existing upper bound for a nonzero
cross-distribution polynomial of degree at most `2g`; the distribution bound is
therefore sharp even without empty dependencies.

There are `2^g` distinguishing masks among `2^(3g)` total masks. Independent
uniform Boolean testing detects a disagreement with probability exactly
`2^(-2g)` per sample. This realizes the established extremal Boolean polynomial
nonzero-mass bound in an entirely withdrawable positive-evidence family. The
general polynomial bound and its monomial examples are prior mathematics; see
[Amireddy et al., ICALP 2025](https://drops.dagstuhl.de/storage/00lipics/lipics-vol334-icalp2025/html/LIPIcs.ICALP.2025.11/LIPIcs.ICALP.2025.11.html).
The determinant identity is also standard. A broader literature comparison is
required before claiming novelty for this particular realization.

## Exact verification

The frozen cohort tests `g=1..5`, using every withdrawal mask: **five families,
37,448 readout comparisons, 62 distinguishing masks and 17,841 jointly unknown
masks**. Every distinguishing mask has size `2g`; no smaller set disagrees.
Every known outcome count is positive. Expanded path-count records agree with
an independent scalar matrix recurrence, exact rational readouts, all polynomial
coefficients and the returned witnesses. All checks pass.

The unchanged `Profile` equivalence checker is used in the three cases within
its width-three limit. Larger cases use explicit record enumeration and the
unchanged polynomial operations; no implementation limit was relaxed. Three
focused tests and all 37 source-prediction tests pass. The proof covers arbitrary
mathematical `g`; computation covers only this finite cohort.

Raw root:
`/Volumes/A/.hms-bench-cache/warm-predictive-matrix-witness-20261103-v1`.
Freeze SHA256:
`b5d29815276c2d24c94732d4b08dd262aea25e39edd8290ab4d2d8c1fca29fff`.
Rational-readout SHA256:
`63c86b8ca7c8a267ab081bfc7971507a868f776b23d89255d0cb5c2bca22b885`.
All seven source files and eight outputs were hash-verified. Receipts:
`matrix-witness-v1-{freeze,finished,summary}.json`.

The practical implication is narrow: testing only small deletion sets cannot
certify arbitrary withdrawal safety, even after ruling out background evidence
and zero-probability outcomes. This supplies neither useful compression on real
tasks nor a measured retrieval or answer-quality improvement. Milestones 3–8
remain open.
