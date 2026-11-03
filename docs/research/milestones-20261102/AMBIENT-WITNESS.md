# Source count sharpens the distribution-witness bound

Consider nonnegative outcome-count profiles over `n` withdrawable sources, with
every record depending on **exactly `g` distinct sources**, `1 <= g <= n`.
If two normalized predictions differ at any withdrawal set where both are
known, they differ at one with at most **`min(2g, n−g)` withdrawals**.
The construction below attains this bound for every mathematical `n >= g >= 1`,
using positive integer counts, binary outcomes and identical unknown states.
All 37,410 computational checks in the frozen finite cohort pass.

For the upper bound, write each outcome count as a degree-g survival polynomial.
A difference between normalized probabilities is detected by a cross product
of outcome and total-count polynomials, whose Boolean multilinear degree is at
most `2g`. Substituting withdrawal variables preserves that upper bound. Möbius
inversion implies that a nonzero multilinear polynomial of degree d cannot vanish
on every Boolean mask of weight at most d. This is standard Boolean interpolation;
see Lemma 2.5 of [Bhrushundi, Harsha and Srinivasan, STACS 2017](https://drops.dagstuhl.de/storage/00lipics/lipics-vol066-stacs2017/LIPIcs.STACS.2017.12/LIPIcs.STACS.2017.12.pdf).
The same inversion proof works over the integer/rational coefficients used here.
A nonzero cross product requires both profiles to be known. Separately, any known
profile retains a complete g-source record, so at most `n−g` sources were removed.
Combining the two observations gives the stated upper bound.

Use the positive matrices from [MATRIX-WITNESS.md](MATRIX-WITNESS.md):

```text
A = [[4,16],[1,19]]    B = [[10,10],[20,20]]    C = [[16,4],[19,1]]
```

A factor with r sources is chosen as follows; variables indicate survival:

| Sources in factor | Count matrix | Boolean determinant | First possible differing mask |
|---|---|---|---|
| r=1 | aA | 60a | 0 withdrawals |
| r=2 | aA+cC | 60(a−c) | 1 withdrawal |
| r>=3, m=r−2 | maA+(b₁+…+bₘ)B+mcC | 60m(a−c)(m−sum bⱼ) | 2 withdrawals |

For the last row, disagreement requires exactly one endpoint to survive and at
least one central source to be withdrawn. Every source contributes positive
integer evidence. A nonempty factor has positive entries; an empty factor is
zero. Thus the two rows of a product of g such factors are known together
exactly when every factor retains a source, and both outcomes are positive then.
Expanding the product gives records with exactly one dependency per factor,
hence exactly g dependencies. There is no background evidence.

For `g <= n <= 3g`, use `floor((n−g)/2)` triple factors, `(n−g) mod 2` double
factors and enough singleton factors to make g factors. Their necessary
withdrawals sum to `n−g`. For `n > 3g`, start with g triple factors and add all
extra central sources to one factor, scaling its endpoint matrices by its new
central-source count m. Every factor still requires two withdrawals, totaling
`2g`. Determinants multiply, so a difference requires all these local differences
at once. Removing the indicated sources attains each lower bound.

This sharp bound concerns **distribution disagreement where both sides are
known**. It does not replace support verification. For example, take uniform
binary observations on all five singleton sources in one profile and only the
first four in the other. Their known distributions always agree; their first
known-versus-unknown discrepancy needs four withdrawals, exceeding `2g=2`.

The 32-family finite cohort uses `g=1..4` and every `n=g..3g+2`, totaling 37,410
withdrawal masks. It compares expanded records, independent scalar matrix
recurrences, exact rational readouts and polynomial coefficients. The existing
width-three checker is reused only within its original limit. Plan:
`benchmarks/research/source_prediction/AMBIENT-WITNESS-PLAN.json`.

The frozen run completed at 2026-11-03T18:01:18Z: 32 families, 37,410 comparisons,
408 distinguishing masks and 14,346 jointly unknown masks. Every first difference
attains `min(2g,n−g)`; all 21 cases within the unchanged checker's width limit also
pass that checker. Four focused tests pass, including the unknown-support
counterexample and additional random small profiles. All nine frozen files and
35 output files were reverified. This computation covers the finite grid; the
algebra above supplies the arbitrary-n/g statement.

Raw root: `/Volumes/A/.hms-bench-cache/warm-ambient-witness-20261103-v1`.
Freeze SHA256: `e1fc05b0a0ffd124037dca9f206f624881eca69450ac429e1c4e343d56f1f61a`.
Readout SHA256: `0b198a3600bd53705a2c11be1ed81166afa4cce485342414a8a9deca068b2479`.
Receipts: `ambient-witness-v1-{freeze,finished,summary}.json` and
`ambient-witness-v1-tests.log`.

This is a derived refinement of the existing restricted-contract result.
Boolean interpolation and determinant multiplication are prior mathematics; a
limited literature search does not establish novelty for the realization.
It supplies no measured storage, retrieval or answer-quality improvement.
