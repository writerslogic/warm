# What future deletion reveals about a winning answer

The earlier [linear quotient screen](QUOTIENT.md) deliberately preserved score
contrasts, which is stronger than preserving only the winner. That left a real
escape route: perhaps a nonlinear encoding could discard those contrasts and
still answer every future query correctly. The following derivation closes that
route for a specific positive association-table contract. It does not establish
an impossibility result for agent memory generally.

Let A be a strictly positive table with at least two rows and two output columns.
A query gives nonnegative row weights w and a surviving-cell mask D. It receives
only `argmax_j sum_i w_i D_ij A_ij`, with ties resolved by the lowest output index.
Any individual cell can be zeroed, and any sequence of those operations is
allowed. For two such tables A and B, identical answers for every w and D hold
exactly when `B = c A` for one positive scalar c.

This is an identifiability statement about the information the contract exposes.
It applies regardless of whether an implementation uses a linear projection,
a nonlinear code, or a learned representation. It does not require the encoding
to store ratios literally. All retained state available to the update/readout
algorithm counts; recovering forgotten information from an uncharged external
dictionary would not evade the statement.

## Derivation

Keep only two cells in different rows and different output columns. Give one row
weight 1 and the other weight t. As t varies, the winning output changes at the
ratio of the two surviving entries. Thus winner-only observations identify that
ratio, without returning scores or a tie set.

Make a graph whose vertices are cells, connecting cells in different rows and
columns. For dimensions at least 2 by 2, that graph is connected except at exactly
2 by 2. Along a spanning tree, the observable ratios determine every entry
relative to one reference entry. For a 2 by 3 table, for example, the graph is a
six-cycle. Additional rows or columns preserve connectivity.

The 2 by 2 exception needs an extra argument. Write the entries as
`[[a,b],[c,d]]`. Two-cell queries determine `r=a/d` and `s=b/c`; those comparisons
alone leave two disconnected components. Three-cell queries connect them:

- If a>b, retaining a,b,d exposes `g=(a-b)/d`. After normalizing a to 1,
  `d=1/r`, `b=1-g/r`, and `c=b/s`.
- If a<b, retaining a,b,c exposes `h=(b-a)/c`. Then
  `b=1/(1-h/s)`, `c=b/s`, and `d=1/r`.
- If a=b, normalization gives both as 1, with `c=1/s` and `d=1/r`.

The sign, including equality, is observable by retaining those triples and
taking the second row's weight sufficiently small and positive. Conversely,
multiplying every entry by one positive scalar multiplies every masked score
by the same scalar and preserves all winners. These two directions establish
the stated equivalence. This is our derivation, not independent peer review.

The intellectual connection is utility identification from choices. Positive
affine uniqueness is established decision theory; see
[Karni's treatment](https://www.econ2.jhu.edu/people/karni/chapter.pdf).
The masking contract here fixes the zero reference and exposes relative scales
that current winners alone conceal. No new general utility-identification theorem
is claimed. The broader literature on comparison-query learning also makes
modeling assumptions explicit, for example
[Learning Linear Utility Functions From Pairwise Comparison Queries](https://arxiv.org/abs/2405.02612).

## Constructive exact check

For integer entries in 1..W, every two-cell ratio and every positive three-cell
gap ratio belongs to the finite set `{p/q: 1<=p,q<=W}`. The reconstructor binary
searches this set using midpoints between adjacent candidate fractions. Therefore
its comparison queries never need to recognize an exact tie. For 2 by 2, the
small sign probe uses weight `1/(W+1)`; a nonzero integer difference cannot be
overturned by that perturbation.

The reconstructor receives only a winner callable, dimensions and W. It never
reads the table or scores. All arithmetic is rational. The plan and source were
sealed before the input enumeration and experiment.

| Input family | Tables checked | Distinct classes up to global scale | Result |
|---|---:|---:|---|
| 2 by 2, entries 1..5 | All 625 | 607 | Every normalized entry recovered |
| 2 by 3, entries 1..3 | All 729 | 727 | Every normalized entry recovered |
| 3 by 2, entries 1..3 | All 729 | 727 | Every normalized entry recovered |
| 8 by 8, entries 1..31 | 32 predeclared seeds | 32 sampled classes | Every normalized entry recovered |

All 2,115 tables passed, using 48,306 winner-only queries. Four regression tests
cover the disconnected 2 by 2 case, both gap signs and equality, rectangular
graphs, global scaling, the smallest bound, invalid inputs and a one-row
counterexample. A single row exposes only value ordering under these masks;
the at-least-two-rows assumption is necessary.

For a finite initial-state family with K different observable classes, any exact
fixed-length binary encoding needs at least `ceil(log2 K)` bits. This is elementary
counting, not a measured storage result. In the two small domains, taking the
global-scale quotient leaves 607 of 625 and 727 of 729 distinguishable states;
the minimum fixed-length bit count is unchanged. This does not imply that a naive
integer array is an optimal physical encoding, nor constrain arbitrary mappings
to ideal real numbers by a linear rank argument.

## Design consequence and limits

Changing a projection into a nonlinear winner code does not remove the information
obligation imposed by this mutation contract. A useful design must exploit actual
structure, restrict the guaranteed query/update family, permit declared error, or
include a charged source representation that supplies information on demand.

This result assumes strictly positive tables, unrestricted nonnegative cues and
arbitrary individual-cell masking. It does not cover structural zeros, a bounded
number of deletions, or whole-row replacement. It also does not cover a deletion
whose caller supplies the old additive contribution: that operation brings new
information to the update algorithm. An episode is not automatically an aggregate
matrix cell. No conclusion about production forgetting, arbitrary natural-language
memory, approximate nearest neighbors, or the possibility of reversible rule
learning follows without proving that its actual contract matches these assumptions.

Source and pre-run plan: `benchmarks/research/mutation_identifiability/`.
Raw cases and every query are at
`/Volumes/A/.hms-bench-cache/warm-mutation-identifiability-20261103-v1`.
Freeze SHA256: `37b5e756103656293c38c4a1fb945dfdb9fe7f28fbfdc18fbb53c7c6a0e21b04`.
Receipts and test/run logs are retained beside this document. No performance,
real-memory-utility, or novelty qualification is asserted.
