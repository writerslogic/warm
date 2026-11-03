# Source-bound transition templates: real-data screen

Separating source identity from reusable control preserves the tested prediction
contract, but fails the predeclared size gate. The best complete packets save only
2.23–3.67% on target-path actions, while exact-coordinate actions grow 0.14–4.88%
against the strongest compressed exact control. This is not an original system
advance or milestone completion.

## Mechanism and contract

A state is eligible when its outgoing observations all have one source and every
active successor has that same source. Mixed/cross-source states and their
predecessors remain an exact fallback graph; reverse propagation computes the
greatest closed eligible subset. Minimize eligible behavior with source names
erased, then retain a source binding for each instantiated control state and an
explicit map from every original state. Terminal states need no binding.

For an instance `(control, source)`, an outgoing template edge inherits the same
source binding at its successor. Withdrawing that source removes all outgoing
evidence and makes actions unknown; any other withdrawal leaves the instance's
transition probabilities unchanged. Ordinary probability refinement preserves
control transitions, so this construction preserves observable action traces for
every withdrawal set. Fallback states retain their original source-tagged edges.
The original-state map, bindings, templates and fallback edges are all charged.

This uses established [nominal/register automata and source-name symmetry](https://drops.dagstuhl.de/storage/00lipics/lipics-vol345-mfcs2025/html/LIPIcs.MFCS.2025.14/LIPIcs.MFCS.2025.14.html)
and [factorized representations](https://fdbresearch.github.io/principles.html).
The contribution tested here is a concrete empirical representation opportunity,
not a claim to have invented register automata. The interface remains the previous
empirical predictive model with a common observable task label. It does not
reconstruct episodes/DOM, support arbitrary future inserts, prove secure erasure,
predict unseen actions or establish model-generated answer quality.

## Fixed controls and results

The same six MiniWoB task/action cells are retained unchanged. All active states
in this cohort satisfy the closed single-source condition; no fallback record is
needed. The candidate can therefore share much more control structure than the
source-identical quotient, making this a favorable test of the proposal.

Controls are both original exact records and the exact source-aware quotient.
Each is encoded as canonical JSON, packed integer rows and hierarchical
delta-coded adjacency, each uncompressed, zlib level 9 and XZ preset 6. Packed
formats use the same original-state hash and action dictionaries; all counts,
bindings and maps are retained. Per-state/action count normalization preserves
all withdrawal-conditioned probabilities. Every packed format is decoded and
compared with its declared model, and compressed bytes round-trip exactly.
The candidate competes with the smallest complete exact control packet.

| Task / actions | Active states | Shared controls including terminal | Best control | Candidate | Change |
|---|---:|---:|---:|---:|---:|
| Date / exact | 2,141 | 1,025 | 110,480 B | 110,632 B | +0.14% |
| Date / target path | 2,141 | 96 | 102,360 B | 99,856 B | -2.45% |
| Large checkboxes / exact | 2,044 | 1,910 | 90,968 B | 95,404 B | +4.88% |
| Large checkboxes / target path | 2,044 | 330 | 78,920 B | 77,160 B | -2.23% |
| Soft checkboxes / exact | 670 | 661 | 34,172 B | 35,564 B | +4.07% |
| Soft checkboxes / target path | 670 | 109 | 29,180 B | 28,108 B | -3.67% |

The gate required at least 20% fewer complete bytes on two distinct tasks and no
cell growing over 5%. It fails the saving requirement. No parameter sweep or
cohort replacement follows. The earlier raw-JSON advantage would have overstated
what this representation adds beyond ordinary compression.

All **105,534 exact rational readout checks** pass, with **48 sequential source
retractions**. Joint refinement of the original and expanded template models also
requires every mapped state to remain equivalent under every future withdrawal.
Six focused tests cover exhaustive small random models, cycles, cross-source
fallback propagation, unknown states, integer/count boundaries and malformed
packed data. This is self-verification, not an independent evaluator's result.

The next unresolved representation question is whether control can be computed
from an observation instead of storing an original-state lookup for every seen
DOM. That would require a learned or inferred observation rule, explicit error
and withdrawal semantics, and unseen observations; it cannot be inferred from
this finite empirical model or obtained merely by omitting the lookup bytes.

## Reproduction

Protocol: `benchmarks/research/source_prediction/FIBER-PLAN.json`.
Implementation: `fiber.py`, `fiber_codec.py`, `run_fiber.py` and two focused test
modules in that directory. Run:
`/Volumes/A/.hms-bench-cache/warm-predictive-source-fiber-20261103-v1`.
Freeze SHA256 `da082abffb2a899f479ff137a51f88125d40e721b883c31b04e5aaa15047f7ed`.
All 23 source, six input and 157 raw-output hashes match. Receipts:
`predictive-source-fiber-{freeze,finished,summary}.json`.
Completed 2026-11-03T13:16:15Z. No paid service or held-out performance query was used.
