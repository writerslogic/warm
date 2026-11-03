# Compression must preserve future mutations, not just current answers

A proposed escape from retaining raw fallback data is to encode only differences
between competing answers. Common score offsets disappear from argmax or softmax,
so they appear disposable. The following exact counterexample rules out that
encoding when deleting a contribution must work from its identity alone.

Consider two association tables, with input cues as rows and competing outputs
as columns:

```
A = [8  8]       B = [1  1]
    [4  0]           [4  0]
```

For any nonnegative normalized cue `(u, v)`, the difference between output scores
is `4v` for **both** tables. Thus every cue has the same winning answer, tie
behavior and margin, and a contrast-only encoding maps A and B to the same state.
This is an algebraic equality of coefficients, not a conclusion from sampled cues.

Now withdraw the contribution at row 0, output 0 by setting that one weight to
zero, without supplying its old weight in the deletion command. With cue
`(1/2, 1/2)`, A's scores become `(2, 4)` and B's become `(2, 1/2)`. The correct
winners differ. The same encoded state and the same deletion command cannot
produce both correct answers. All input weights are nonnegative, and both
withdrawn contributions existed before deletion.

This concerns deleting one contribution to an output that remains eligible,
not masking that output entirely. It is consistent with the narrower operator
semantics in ASSOCIATION.md; it is not a claim of document erasure. The current
tensor probe retains the raw table and therefore does not have this ambiguity.

## Necessary condition for the next candidate

If encoding `E` admits a deterministic update `U` with exact rebuild equivalence,
then for every supported mutation `d` it must satisfy
`U(E(S), d) = E(d(S))`. Consequently, `E(S) = E(T)` requires
`E(d(S)) = E(d(T))`. Even when only readout equivalence is required, the two
updated encoded states must answer every allowed query identically. An encoding
that preserves today's answers can fail this condition.

The general ideas are established. Database view updates and complementary
information have a long history, including
[Bancilhon and Spyratos, Update Semantics of Relational Views](https://cs.uwaterloo.ca/~david/cs848s18/constant-complement.pdf)
and [Cosmadakis, Translating Updates of Relational Database Views](https://dspace.mit.edu/entities/publication/052afe61-a13c-43b5-835a-176717d7b2cb).
That literature studies related translation questions; it is not a claim that
this association-table witness or every deletion contract is its theorem.
We make no novelty claim for the condition or the elementary counterexample.

The more precise research question is now: **what additional information makes
a compressed association state sufficient for its allowed future changes, and
can that information remain substantially smaller than the strongest exact
representation?** Charge a retained complement, a source lookup or a deleted
value supplied by the caller explicitly. A hash or a tombstone cannot recreate
discarded weights. This narrows the search before another compression prototype;
it does not establish that no useful restricted encoding exists.

The [executable witness](../../../benchmarks/research/certified_association/mutation_witness.py)
uses exact rational arithmetic, checks the equal coefficients, verifies a cue grid
as an additional sanity check, and asserts the divergent post-deletion winners.
Run `uv run --no-project python benchmarks/research/certified_association/mutation_witness.py`.
Its output is retained in `mutation-witness.json`.
