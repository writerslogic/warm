# Predictive memory on recorded task histories

The first natural-data screen finds an obstacle to the exact predictive quotient:
every observed state/action pair has evidence from only one demonstration.
Preserving whether evidence becomes unknown after even one source withdrawal
prevents almost all active-state merging on the two checkbox tasks. The date task
retains a possibility requiring full probability/successor refinement. This is
exposed development evidence, not a system advantage, agent result or novelty claim.

## Inputs and recording semantics

The [official documentation](https://miniwob.farama.org/content/demonstrations/)
identifies the human demonstration repository and capture/bubble event semantics.
The manifest pins `stanfordnlp/miniwob-plusplus-demos` revision
`6506e1a444f32af99c487bd59be5b925476d9108`. Before reading cohort contents,
repository tree metadata selected the first three alphabetically ordered task
names with at least 128 eligible files, then the first 128 paths per task.
No success filter or replacement is allowed. A separate `choose-date-easy` file
was inspected only for schema. All 384 compressed Git blob hashes and SHA256s match.

Only adjacent capture/bubble events with matching action dictionaries become
transitions. The audit preserves **7,863 paired transitions, 128 unmatched events
and 21 empty demonstrations**. Missing outcomes are never inferred. Five date
traces and 16 soft-checkbox traces are empty. Unknown empirical action evidence
does not establish impossibility in the environment; overall episode reward does
not supply a missing event outcome.

State identity includes task, instruction and full recorded DOM. Generated
`recordingTarget` markers are removed. Negative references on text pseudo-elements
are normalized while retaining text, tree order, geometry, positive element
references and other fields. The official
[recorder source](https://github.com/Farama-Foundation/MiniWoB-plusplus/blob/33c3b4ddef8c6eb67c57a29663d844b1eda7e614/miniwob/html/core/core.js)
allocates new negative text references on each observation, whereas real elements
retain their references. This explains the observed changes; the precise 2017
recorder revision is unknown. No task content is abstracted away.

Two declared action representations are retained: exact event dictionaries, and
an event-target child path with coordinates removed when the target is unique.
The latter is a separate projection, not an equivalence claim about coordinates.

## Necessary support condition

For a state/action pair with source set `S`, the empirical transition becomes
unknown exactly when all of `S` is withdrawn. Unrestricted-withdrawal equivalence
therefore requires identical action-to-source-support maps. For budget `k`, sets
larger than `k` are always nonempty; smaller sets must still match. Exhaustive
three-source tests verify this characterization.

Grouping by this condition gives an **upper bound** on active-state merging.
Probability and successor constraints can only reduce it. All states within each
task share an observable label for this optimistic bound; preservation of
instructions/DOM as output labels is not promised. States without observed outgoing
transitions are reported separately to avoid inflating the active-state result.

| Task | Active states | Maximum merges, exact actions | Maximum merges, target-path actions |
|---|---:|---:|---:|
| choose-date-nodelay | 2,141 | 895 | 1,107 |
| click-checkboxes-large | 2,044 | 0 | 4 |
| click-checkboxes-soft | 670 | 0 | 1 |

These bounds are identical for one, two and unlimited withdrawals: every observed
state/action pair has exactly one source. Without withdrawal, target-path
support-only bounds permit 2,052, 1,715 and 562 merges respectively. This difference
measures a semantic restriction, not bytes or a valid predictive quotient.
The 798/102/108 states without outgoing observations are excluded from the table.

## Corrections and reproducibility

Version 1 rejected empty demonstrations; its failure is preserved. Version 2
retained them but used generated text references in state identity, spuriously
making every captured DOM distinct. Its raw results must not be used to claim
lack of recurrence. Version 3 corrects identity on the same exposed cohort.
Five focused tests pass, covering unpaired events, source-support equivalence,
identity boundaries and input limits. No qualification cohort was opened/reselected.

Raw runs: `/Volumes/A/.hms-bench-cache/warm-predictive-miniwob-20261103-v{1,2,3}`.
Version 3 freeze SHA256:
`d25f5b82768bb2ebe71d9e553eaa4a368f5ce4d947322dc906d419be111d58c0`.
All four source files, 12 raw outputs and 384 input hashes were verified.
Plan/code: `benchmarks/research/source_prediction/`. Receipts here:
`predictive-miniwob-v3-{freeze,finished,summary}.json` and test/run logs.

## Complete refinement and storage accounting

The follow-up runs complete singleton-source refinement on all six cells, using
the same empirical interface. It includes the state dictionary and original-state
map in the retained representation, and uses identical action dictionaries and
dense integer record encoding for baseline and quotient. The three new tests
compare 100 small models against exhaustive-withdrawal refinement and compare
quotient repair with fresh minimization for all 16 withdrawals per model.

| Task / action representation | Active states after safe refinement | Baseline bytes | Quotient bytes |
|---|---:|---:|---:|
| Date / exact | 1,254 / 2,141 | 321,393 | 301,456 |
| Date / target path | 1,042 / 2,141 | 262,547 | 237,835 |
| Large checkboxes / exact | 2,044 / 2,044 | 327,493 | 336,601 |
| Large checkboxes / target path | 2,040 / 2,044 | 211,826 | 220,813 |
| Soft checkboxes / exact | 670 / 670 | 122,581 | 125,126 |
| Soft checkboxes / target path | 669 / 670 | 77,205 | 79,730 |

All **105,534 exact rational state-readout checks and 48 sequential retractions**
pass. Each sequence removes source indices cumulatively through 1/2/4/8/16/32/64/128,
re-minimizes the retained quotient, and matches fresh minimization from all
surviving observations. No original discarded record is consulted by repair.
These bytes cover the predictive interface, not original DOM/episode recovery,
arbitrary future inserts, process memory or a production memory system.

The date task saves only **6.20–9.41%**; the checkbox tasks grow **2.08–4.24%**.
Ordinary current-only merging forms fewer blocks but violates withdrawal semantics;
it is retained as an unsafe diagnostic, not presented as a matched control.
The exact mechanism therefore has no useful general memory advantage demonstrated
by this cohort. Enlarging constructed algebraic examples would not resolve this.

Run: `/Volumes/A/.hms-bench-cache/warm-predictive-miniwob-state-20261103-v1`.
Freeze SHA256:
`33dfe4c93b611a17e60e5d66742384336993aae257c945d2a396403b6b65c569`.
All eight source, six normalized input and 19 raw output hashes were verified.
Receipts: `predictive-miniwob-state-{freeze,finished,summary}.json` and test/run logs.
Learning a more general abstraction needs an explicit approximation contract and
tests of unseen behavior; this exact empirical screen does not establish it.

## Do not mistake source renaming for a new solution

A tempting response is to merge shared transition shapes while keeping source
identities in separate registers. That may avoid requiring identical support
sets in every merged control state, but registers, bindings and transitions must
still preserve each original withdrawal query. Merely renaming sources is not
an invention: [nominal/register automata with permutations](https://drops.dagstuhl.de/storage/00lipics/lipics-vol345-mfcs2025/html/LIPIcs.MFCS.2025.14/LIPIcs.MFCS.2025.14.html)
already separate finite control from data names and formalize their symmetries.
[Factorized databases](https://fdbresearch.github.io/principles.html) already share
repeated expressions and factor provenance. These are necessary controls for a
future mechanism, not a basis for claiming that a familiar representation has
moved the frontier. A subsequent [closed-source template screen](PREDICTIVE-FIBERS.md)
now implements this restricted factorization and compares it with compressed
exact records and quotients. It preserves every checked readout but saves only
2–4% on target-path actions, failing its frozen 20% gate; ordinary compressed
controls remove most of the apparent opportunity.
