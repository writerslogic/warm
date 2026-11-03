# A sharp deletion bound with positive evidence

The `2g` distribution-witness bound in [SOURCE-PREDICTION.md](SOURCE-PREDICTION.md)
cannot be reduced merely by requiring positive evidence counts, positive outcome
probabilities, or an answer after every withdrawal. Two admissible profiles can
agree after every set of fewer than `2g` withdrawals and differ after exactly
`2g`. This is a mathematical result for the stated evidence contract; it supplies
no measured storage, latency or answer-quality advantage.

The later [matrix construction](MATRIX-WITNESS.md) also attains `2g` when every
observation has exactly `g` nonempty, withdrawable dependencies. It closes the
gap left by the anchored variant below; this earlier experiment and its portable
bundle remain unchanged.

## Construction and proof

For any integer `g >= 1`, take two disjoint groups `X,Y`, each containing `g`
Boolean source-survival variables. Let `P,Q` be the sums of all even-cardinality
and odd-cardinality subset monomials in `X`, respectively. Let `S,R` be the same
even and odd sums in `Y`. Include the empty monomial `1` in each even sum.
Every monomial represents an observation whose dependencies must all survive.

Construct two binary outcome-count vectors:

```text
(A,B) = (P+R, Q+R)
(C,D) = (P+S, Q+S)
```

Every coefficient is a nonnegative integer and every monomial has degree at
most `g`. Their determinant factors:

```text
AD - BC = (P-Q)(S-R)
        = product(1-x_i for i in X union Y).
```

Apply the same positive mixing matrix to both outcome vectors:

```text
left  = (2A+B, A+2B)
right = (2C+D, C+2D).
```

The determinant becomes `3 * product(1-x_i)`, since the mixing matrix has
determinant three. Mixing changes only outcome multiplicities, so maximum
dependency width remains `g`. All represented observations have positive integer
multiplicity. Both resulting outcome counts are strictly positive for every
withdrawal: `A >= 1` and `C >= 2`, and the mixing coefficients are positive.
There is therefore no unknown-state discrepancy or zero denominator.

Two binary normalized distributions are equal exactly when their count-vector
determinant is zero. The product vanishes whenever any of the `2g` sources
survives. Only withdrawing all of them distinguishes the profiles. At that point
the distributions are `(2/3,1/3)` and `(5/9,4/9)`. Combined with the existing
degree-based upper bound, this proves sharpness for the contract allowing empty
dependency sets, which represent retained background evidence.

To remove empty dependencies, add a common withdrawable anchor to every
observation. Maximum width becomes `w=g+1`. With the anchor present, exactly
`2g=2w-2` other withdrawals are necessary; with it absent, both profiles are
unknown. This establishes a `2w-2` lower bound for that variant. It does **not**
establish sharpness of `2w` when all dependencies must be nonempty.

## Exhaustive evidence

The frozen cohort contains unanchored `g=1..6` and anchored `g=1..5`, with every
withdrawal mask and no excluded case. All **11 families / 8,188 readout
comparisons** pass. Record enumeration agrees with an independently calculated
parity-count formula; all cross-polynomial coefficients and unique witnesses
match the proof. The unchanged profile checker also returns the predicted
witness in all five cases within its declared width-three limit. Three focused
tests pass; the complete source-prediction suite passes 34 tests.

Raw root:
`/Volumes/A/.hms-bench-cache/warm-predictive-witness-tightness-20261103-v1`.
Source freeze SHA256:
`0a6e2f66a35fbfd85b00f0c76c213e63b615de5e331564c2a3312d7e642eef9b`.
Raw rational-readout SHA256:
`b4968ff6e635aa708c7aca4a82dbfbf486c0595c2f1b7b65900df84aa226c767`.
All five frozen source files and fourteen output hashes were verified.
Receipts: `predictive-witness-tightness-{freeze,finished,summary}.json`.

## Implication and prior art

Testing only small deletion sets cannot certify general equivalence, even with
strictly positive answers. For this unanchored family, independent uniform
Boolean masks detect the discrepancy with probability exactly `2^(-2g)` per
sample. Exhaustive sampling success rates can therefore be misleading unless
the withdrawal distribution and dependency width are stated.

The general Boolean polynomial nonzero-mass bound and tight monomial examples
are established mathematics; see the introduction of
[Amireddy et al., ICALP 2025](https://drops.dagstuhl.de/storage/00lipics/lipics-vol334-icalp2025/html/LIPIcs.ICALP.2025.11/LIPIcs.ICALP.2025.11.html).
Our derivation realizes that extremal behavior inside the declared positive
evidence-profile contract. A broader literature review is still needed before
claiming that realization as a novel theorem. The algebraic proof applies to
all `g >= 1`; computation verifies only the finite cohort above.
