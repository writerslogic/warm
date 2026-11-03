# Original research direction

The user's 2026-11-02 instruction changes the research priority: competitive
reproduction supplies controls; it is not the research objective. Seek mechanisms
that change the attainable cost/quality tradeoff. The eight qualification gates
remain necessary evidence for a claimed advance. No advance is established yet.

The latest exact result is the [source-count refinement](AMBIENT-WITNESS.md):
with exactly g dependencies per record, normalized distributions have a sharp
`min(2g,n−g)` withdrawal-witness bound. Positive integer evidence with matching
unknown states attains every source-count regime. The proof extends the matrix
construction, and all 32 finite families / 37,410 checks pass; a portable bundle
reproduces 34 deterministic outputs. This establishes a restricted mathematical
constraint, with novelty and useful system gains still unestablished.

## Scientific questions

1. **Can information access replace similarity as the navigation objective?**
   The historical trace attributes 1,268 of 1,308 missing-neighbor paths to a
   vertex evicted before expansion. Similarity scores a candidate as an answer;
   it does not directly measure the candidate's value as an access point. Test
   whether uncovered and rare-neighbor mass predicts that value at a fixed
   search budget. This is a hypothesis about the mechanism, not an invention
   claim. The frozen first falsifier is
   [INFORMATION-PLAN.json](../../../benchmarks/research/vector_topology/INFORMATION-PLAN.json).
   Its deliberately optimistic candidate-coverage ceiling can reject weak
   proxies before paying for a native implementation. A positive result still
   needs actual scoring, total-compute accounting and current shipping controls.
   **Result:** the first three degree-based proxies all failed the frozen rule;
   see [INFORMATION.md](INFORMATION.md). Do not promote or tune them to these cells.

2. **Can associative storage cancel interference selectively without decoding
   the entire memory?** The repository's additive phase summaries estimated
   density, which loses rare nearest neighbors even with an exact kernel. Its
   spatially coupled decoder recovered synthetic labels, but retained about
   225 MB of matrix for 264 bytes of labels. Those are failures of the useful
   system objective. A research candidate must jointly supply query addressing,
   local interference removal and exact update/forget semantics while charging
   every codebook, decoder state and retained source byte. The first theoretical
   obligation is to identify assumptions under which a query retrieves a sparse
   subset without a full scan or hidden linear-size dictionary. Then test rare
   needles, dense distractors, correlated keys, deletion and adversarial aliases
   against direct sparse indexes at equal whole-process memory. Ordinary
   matrix-free transforms alone would be implementation of established coding
   theory, not an original contribution.

3. **Can memory retain the evidence needed to distinguish competing answers?**
   Instead of minimizing summary length alone, ask for a small, provenance-bearing
   certificate that preserves the answer and its uncertainty across specified
   temporal queries and counterfactual updates/deletions. A possible contribution
   would be an incremental certificate with a precise invalidation guarantee,
   including conflicting and withdrawn evidence. Temporal tables, event sourcing,
   state compilation and source citations already exist; combining their names
   is insufficient. First specify the query family, the competing-answer model
   and certificate validity. Compare with a strong indexed temporal store plus
   exact evidence selection before any LLM experiment. Failure to save at least
   30% reader tokens with identical supported answers/provenance rejects the
   context advantage on that workload. Extraction cost and retained-source
   storage must be included, and synthetic success cannot qualify milestone 4.

These are separate hypotheses. Do not force them into one architecture before
an experiment supports the connection. Preserve failed hypotheses and articulate
what each result rules out. Do not expose held-out qualification inputs to tuning.

## Prior art boundaries checked on 2026-11-02

- Distance-based adaptive beam termination is established by
  [Al-Jazzazi et al., NeurIPS 2025](https://proceedings.nips.cc/paper_files/paper/2025/hash/904fe070f484231aa26dbdb37816cd40-Abstract-Conference.html).
  Query-distribution-based ef selection is addressed by
  [Zhang and Miller, Ada-ef](https://arxiv.org/abs/2512.06636).
  Neither ordinary adaptive ef nor increasing a reservoir is an invention here.
- Query-aware graph pruning is established by
  [Zhang et al., GraSP](https://www.microsoft.com/en-us/research/publication/grasp-optimizing-graph-based-nearest-neighbor-search-with-subgraph-sampling-and-pruning/).
  Bridges and multi-graph traversal also have prior art, including
  [Wang et al., ICCV 2013](https://openaccess.thecvf.com/content_iccv_2013/papers/Wang_Fast_Neighborhood_Graph_2013_ICCV_paper.pdf).
  Current WARM already implements two-hop search and a reservoir. Unseen-neighbor
  proxies need a broader literature comparison before any novelty assertion.
- Sparse and structured associative retrieval is studied by
  [Santos et al., JMLR 2025](https://www.jmlr.org/papers/v26/24-1961.html).
  LSH/kernel sketches with compressed-sensing recovery are established by
  [Coleman et al., ICML 2020](https://proceedings.mlr.press/v119/coleman20a.html).
  Sparse superposition coding, spatial coupling and fast Hadamard operators are
  established by [Barbier and Krzakala](https://arxiv.org/abs/1503.08040).
  Those results motivate controls, not claims of ownership of their mechanisms.
- State/relation/count compilation with source text is already proposed by
  [Wu et al., Task-Oriented Memory Compilation](https://research.birmingham.ac.uk/en/publications/task-oriented-memory-compilation-executable-state-representations/).
  Any proposed evidence certificate must distinguish itself technically from
  compiled state and existing provenance/incremental database methods.
- Minimizing provenance is itself established, including
  [Amsterdamer et al., PODS 2011](https://www.cs.tau.ac.il/~danielde/publications/pods2011a.pdf).
  A smaller witness set alone is not a novel memory mechanism.

## A fast-transform idea ruled out as a novelty claim

Consider a Walsh codebook for each factor, with an independent row permutation
and a sign mask. Cleanup could use a fast Walsh transform instead of retaining
and scanning every codeword. Independent permutations appear to avoid the obvious
XOR aliasing of a shared Walsh basis. However, this remains a binary linear code:

\[
c_f(j,\ell_f)=(-1)^{s_f(j)+\langle p_f(j),\ell_f\rangle},\qquad
\prod_f c_f(j,\ell_f)=(-1)^{\sum_f s_f(j)+\sum_f\langle p_f(j),\ell_f\rangle}.
\]

All exponents are over GF(2); each permuted row index and factor index is a bit
vector. Removing the known masks gives one linear system in the concatenated
factor-index bits. If its generator has full column rank, noiseless factorization
is linear algebra; if not, collisions remain. Restricting a codebook to a subset
of columns adds a membership check, not a new encoding principle.

This derivation is our analysis of the candidate. The broader construction and
factorization of HDC representations with linear codes is already studied by
[Raviv, Linear Codes for Hyperdimensional Computing](https://arxiv.org/abs/2403.03278).
Do not build a resonator around this construction and label its storage saving
an invention. Noise, superposition, query addressing and deletion would require
separate treatment and comparison with the linear-code decoder, not just a dense
random-codebook baseline. This screening avoided another implementation that
would mainly reproduce established work.

## Next candidate: structured compression with explicit rare-fact guarantees

Another scientific route is tensor-network compression: represent relationships
between roles as a fact tensor, then contract it against partial query cues.
Tensor trains and streaming approximations are established tools, including
[Kressner et al.](https://arxiv.org/abs/2208.02600); approximation error analysis
also exists, including [Qin et al.](https://arxiv.org/abs/2207.04327).
Using them alone would not meet the originality objective.

The specific unresolved question is whether query-local error certificates and
explicit residual facts can preserve rare evidence through updates and forgetting
while the combined representation is substantially cheaper than strong exact
indexes. This is a candidate question, not a claim that the combination is new.
The narrower continuous-association probe described below does not resolve it.

Let exact fact counts be T, a structured approximation be L, and explicit
corrections be S. For a query tensor Q, the contraction error obeys
`|<T-L-S,Q>| <= ||T-L-S||_F ||Q||_F`. This elementary Cauchy–Schwarz bound is a
baseline, not a new theorem. A global bound may be too loose for any useful
certification; low average reconstruction error can hide every rare fact. A
candidate needs inexpensive local bounds or another stronger argument, and must
abstain or read exact evidence when the bound cannot separate competing answers.

Before a native implementation, freeze a small screen with correlated relations,
rare exceptions inside common clusters, independent random facts, temporal
conflicts and repeated erase/reinsert cycles. Compare with packed bitmaps,
compressed sparse postings and a compressed exact serialization, not only a
dense tensor or random codebook. Include source/provenance payloads, correction
indexes, all factor cores, reconstruction scratch and retained originals in
memory accounting. Supplying the generator's latent factors to the candidate
would invalidate the comparison; infer its representation from the same observed
facts available to the controls.

The first acceptance test is zero false or missing facts across the declared
queries and mutation sequence, including rare cases, with at least 50% lower
combined retained bytes than the best exact control on two declared structured
workloads. This small screen would only justify larger native experiments;
whole-process memory, independent workloads and both physical configurations
still govern milestone 6. Report the random-fact regime even if no compression is
possible. Do not infer secure erasure from a negative correction that leaves a
deleted fact encoded in an old factor; forgetting requires a separate retained-
state analysis and rebuild-equivalence check.

## Continuous-association falsifier: rejected

A frozen operator probe tested inferred tensor factors, explicit rare corrections
and guarded exact fallback on continuous soft-cue readout. This does not implement
the discrete fact-storage hypothesis above. The corrected experiment preserved
every tested winner but failed the predeclared 2× gate in all 12 target cells;
rare exceptions were slower than the exact control. Version 1's apparent win was
invalid because a layout conversion copied the control's full tensor per query.
The corrected version retained the same input hashes, seeds and acceptance rule.
See [ASSOCIATION.md](ASSOCIATION.md) for all outcomes, storage costs and limitations.

Do not promote this implementation or rerun parameter sweeps to rescue its gate.
Keeping raw input for fallback prevents total-storage savings. A small error in
continuous scores does not establish rare-fact or provenance preservation. Those
are the unresolved scientific constraints, independently of Python overhead.

[MUTATION.md](MUTATION.md) makes one constraint concrete: two association tables
can have identical answers and margins for every cue, yet require different
answers after the same contribution deletion. The exact-arithmetic witness
rejects discarding common score offsets without accounting for the information
needed by future updates. This connects the research question to established
database view-update/complement theory; it is analysis, not an invention claim.
The next candidate must explain how its compressed state supports allowed future
mutations before it earns a performance experiment.

[QUOTIENT.md](QUOTIENT.md) now tests that requirement with exact observability
closure. Whole-row changes preserve a valid score-contrast quotient; individual
cell changes force full state rank in every declared case. All exact identities,
exhaustive small-state readouts and update/rebuild checks passed. This uses known
control theory and identifies a permitted restricted encoding, not a new theorem
or a product advantage. The next contribution would need a scalable mechanism
for a real query/update/provenance contract beyond existing incremental views.

The literature check is a bounded initial search, not an exhaustive novelty
review. Research claims require a precise difference, reproducible evidence and
disclosure of where established methods remain stronger.

## Further screening on 2026-11-03

The next question is whether a compact decision memory can remain sufficient
after source corrections or deletions. It must track the changed full-memory
decision, rather than preserve a now-obsolete answer. For a retained subset S,
full evidence M and allowed deletion sets D, the relevant contract is
`F(S minus D) = F(M minus D)` for every declared D. A static witness only checks
D empty. This is a proposed research contract, not an established new algorithm.

Several superficially promising routes already have close prior art:

- [JointCore](https://arxiv.org/abs/2610.23266) searches for jointly sufficient
  evidence with group deletion. Merely extending singleton selection to groups
  would repeat it.
- [WiCER](https://arxiv.org/abs/2605.07068) already applies counterexample-guided
  refinement to memory compilation. A compile/test/repair loop alone is not our
  contribution.
- [Deletion-robust coresets](https://arxiv.org/abs/2203.01241) retain alternatives
  against adversarial removal; keeping backup witnesses is already a studied
  strategy. Its submodular approximation objective differs from exact changed
  decisions, but that difference alone does not establish novelty.
- [Kinetic data structures](https://graphics.stanford.edu/~comba/papers/socg.pdf)
  maintain compact certificates and repair them when predicates fail. Calling
  evidence dependencies certificates would not invent this mechanism.

The public JointCore implementation was inspected at commit
`96eeb06a8dbac601bbb5213e7154d6ce40dce3b5`. Its exact oracle is a monotone
presence function over at most four designated support records. In particular,
the XOR family requires all support records to remain known, and the stale-policy
family is also a support-presence conjunction in that oracle. Horizon is a label
in the paper task generator, not a long revision sequence. Consequently, a
bounded-deletion extension of those exact tasks would mostly measure familiar
backup selection. We have not run or relabeled that extension as real memory
utility or an original advance.

The candidate therefore needs a stronger test: conflicting evidence with actual
state changes, explicit source withdrawal, and a concrete algorithm that reduces
repair or reader work compared with an indexed temporal store and incremental
provenance. Raw-episode retention remains a required control: the consolidation
failures reported by [Zhang et al.](https://arxiv.org/abs/2605.12978) make summary
size alone an inadequate success criterion. No paid model calls or third-party
benchmark execution were used in this screening.

[IDENTIFIABILITY.md](IDENTIFIABILITY.md) closes a specific gap in the earlier
linear argument. For a strictly positive association table, arbitrary individual-
cell deletions and unrestricted nonnegative cues make every entry ratio observable
even when the interface returns only a winning label. A winner-only rational
reconstructor recovered all 2,115 predeclared tables, including the special 2 by 2
case, in 48,306 queries. This limits nonlinear encoding as an escape from that
contract; it does not rule out structured data, other mutation semantics,
approximation or reversible rule learning. The derivation connects to established
utility identification and is not labeled an invention.

[SOURCE-LIFT.md](SOURCE-LIFT.md) supplies a constructive next step: couple each
equation to an algebraic encoding of its source dependencies, then retain a basis
of the lifted equations. Contraction can erase any bounded set of sources while
preserving the exact span of the surviving evidence. The bound is
`rank * binomial(withdrawals + provenance-width, provenance-width)`, attained by
the declared subset-certificate examples. This uses established representative-
family mathematics; it is a concrete mechanism for investigation, not a priority
claim. All 6,000 exhaustive cases / 22,713 states, 30 tight constructions and 12
larger cases passed correctness, including rare evidence and contradiction removal.
The size screen failed because the strong exhaustive-deletion control was smaller.
The next question is cheaper construction of equally small source-aware
certificates, with realistic evidence extraction still a separate obligation.

[SOURCE-REFINEMENT.md](SOURCE-REFINEMENT.md) investigates that construction cost.
Proof-directed pruning supplies an explicit withdrawal witness for every retained
constraint. An early gain against exhaustive enumeration exposed a stronger,
simpler control: recurse only on sources in the current greedy basis. The first
256-source check used six basis calculations to reproduce the identical union
certificate that exhaustive enumeration obtained from 32,897 withdrawal sets.
The full cohort preserved all 209,868 tested withdrawal states. The stronger
control produced byte-identical certificates with only 6–116 basis calculations
per case and eliminated the lift's apparent speed advantage in all 12 cells.
No system advantage or novelty is qualified. This proof-guided direction is the
current constructive lead, with the unnecessary algebraic overhead and weaker-
control speedups retained as research evidence rather than advertised as
breakthroughs.

[TEMPORAL-TRACE.md](TEMPORAL-TRACE.md) tests the next semantic obstacle: source
withdrawal can revive an older fact, so positive proof retention alone is unsafe.
A bounded execution-read certificate includes false guards and preserves nonlinear
decisions, historical snapshots, contradictions and original read provenance.
All 3,932 cases / 86,846 snapshot-withdrawal checks passed, with byte-identical
agreement to exhaustive trace union. Its frozen size screen failed: independent
guard sources gave 84% smaller packets, but a shared dependency reduced savings
to 7–13%, and unconditional reads gave none. This identifies a concrete boundary:
the structure of possible source failures governs useful compaction. Dynamic
slicing and dependency provenance are established prior art; query-specialized
symbolic compilation and real reader utility remain required controls, and the
result does not qualify a new memory architecture or a system advantage.
The subsequent ordered-decision-diagram control produced identical packets in
all 12 temporal cohort cells and was faster in every development timing. Thus
the apparent independent-source saving is available through established symbolic
compilation too. Preserve the temporal correctness result and source-correlation
boundary, but do not promote this compiler as the original advance being sought.

[SOURCE-PREDICTION.md](SOURCE-PREDICTION.md) opens a different route: predictive
models that remain equivalent to surviving observations after source withdrawal.
Boolean polynomial identities plus an explicit nonempty-evidence condition decide
exact histogram equivalence for unlimited withdrawals. The first 8,597-pair /
81,120-withdrawal screen passes, including a counterexample showing why bounded
distribution checks alone miss loss of all evidence. This is a prerequisite for
retractable state abstraction. Strong symbolic controls, complete state refinement
and a real behavioral workload remain required; established parametric model
checking prevents treating symbolic equivalence itself as an invention.
The complete state-refinement screen now preserves 16,908 checked transitions and
1,074 sequential withdrawal steps across 108 models, with independent partition
enumeration and both symbolic controls agreeing. It compacts designed correlated
histories but cannot compact the independent-history stress case. Real behavioral
evidence, rather than a larger version of the constructed cases, must determine
whether this mechanism offers a useful advance.
The [first recorded-task screen](PREDICTIVE-MINIWOB.md) now tests that boundary
on 384 human demonstrations. Every observed state/action pair has one source;
even one permitted withdrawal prevents almost all possible active-state merging
on both checkbox tasks. A date-task opportunity survives the necessary support
test. Full refinement now preserves all 105,534 checked readouts and 48 sequential
retractions, but dictionary/map accounting leaves only 6–9% date-packet savings
and 2–4% growth on the checkbox tasks. Empty traces, unmatched events and the
correction of generated text-reference identities remain in the evidence. This
route has not demonstrated the useful general memory advantage being sought.

The subsequent [source-bound template test](PREDICTIVE-FIBERS.md) separates
reusable control from source identities instead of requiring identical sources
in every merged control state. All real-trace active states meet its closed
single-source condition, and 105,534 readout checks / 48 retractions remain exact.
Despite that favorable structure, packed and compressed exact controls reduce
the saving to 2–4% on target-path actions, with growth on coordinate actions.
The 20% complete-byte gate fails. This uses known register symmetry; further
progress needs an observation-to-control rule that generalizes without a stored
lookup for every recorded state, including a defensible withdrawal contract.

## Sparse decoding also requires a distinct mechanism

Another candidate was to replace dense associative measurements with implicit
sparse hash equations, then decode a query-local neighborhood by peeling known
items and subtracting interference. Its ordinary building blocks are established:
[invertible Bloom lookup tables](https://arxiv.org/abs/1101.2245) already support
updates, deletion, lookup and content recovery below a load threshold, and
[Mitzenmacher and Morgan](https://arxiv.org/abs/1807.09694) already combine them
with locality-sensitive hashing for robust geometric set reconciliation.
[Listing guarantees](https://arxiv.org/abs/2212.13812) and
[reduced memory/randomness constructions](https://arxiv.org/abs/2306.07583)
are also existing research. No implementation or novelty claim follows merely
from combining these terms. A distinct candidate would need a concrete
query-local recovery guarantee under unrelated distractor load, count every
payload/addressing structure, and beat direct sparse postings plus these controls.
That technical mechanism has not yet been supplied.


## A sharp constraint for withdrawal-safe predictions

[WITNESS-TIGHTNESS.md](WITNESS-TIGHTNESS.md) supplies a new derivation in this
investigation: positive evidence can attain the worst-case `2g` deletion-witness
bound, with both outcomes strictly positive after every withdrawal. The profiles
agree for every smaller withdrawal set. All 11 frozen families / 8,188 exact
comparisons pass. The later [matrix construction](MATRIX-WITNESS.md) closes the
nonempty-dependency gap: it attains `2g` using observations with exactly `g`
withdrawable dependencies and no background evidence. Five families / 37,448
exact comparisons pass, including matching unknown states and strictly positive
known outcomes. Portable proof/source/raw result bundles are prepared for the user's run in
[REPRODUCTION.md](REPRODUCTION.md).

This realizes an established extremal Boolean polynomial inside the particular
positive-evidence contract. It establishes a testing limitation, not a useful
memory architecture or an invention claim. The implication for future work is
specific: positivity does not justify certifying withdrawal safety by testing
only small deletion sets or a few random masks. A useful abstraction must exploit
additional structure, supply an exact certificate, or state an approximate
contract with a measured error bound.

The next observation-to-control proposal also needs a technical distinction from
known [causal state abstractions](https://proceedings.mlr.press/v162/wang22ae.html)
and [causal representation learning for state abstraction](https://arxiv.org/abs/2401.12497).
Likewise, treating graph search as descent on an energy landscape alone does not
distinguish it from existing [monotonic relative neighborhood graphs](https://arxiv.org/abs/2107.13052).
These are prior-art boundaries, not implemented results or rejected experiments.

## Per-query retrieval certificates: additional prior art

A broader certificate search on 2026-11-03 found direct predecessors that any
future proposal must address. [Certified Cosine](https://arxiv.org/abs/1910.02478v2)
already derives correctness certificates for high-dimensional nearest-neighbor
search. [CertEx](https://arxiv.org/abs/2610.21325v3) describes conservative
low-precision GPU screening followed by exact evaluation of survivors.
[Lychee](https://arxiv.org/abs/2610.30697v1) applies certified screening within
probed inverted lists; its stated exactness is relative to those lists, rather
than proof that no unprobed list contains a true neighbor. Their reported
performance is the authors' evidence, not a reproduced result in this project.

Consequently, graph seeding, interval exclusion, selective exact scoring, or
adding the word "certified" would not by themselves supply the distinct mechanism
the user requested. The failed arithmetic screen remains a development result.
A proposed advance must identify its additional guarantee or cost reduction,
include certificate construction and maintenance, and compare with these relevant
controls where the execution resources match. The ongoing frozen qualification
retains its original methods and gates; no new mechanism is tuned against its
now-exposed query vectors.
