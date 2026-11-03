# Source withdrawal as algebraic projection

This is a positive construction for a restricted memory contract, followed by an
exact experiment. It uses established representative-family algebra. It does not
establish mathematical priority, competitive superiority or agent answer quality.

The question is whether a small collection of original, source-backed equations
can preserve every conclusion after sources are withdrawn. One source may support
many equations; one equation may require several sources jointly. This differs
from keeping a summary that answers today's questions, or preserving an obsolete
answer after its evidence disappears.

## Contract and construction

An evidence row is an augmented equation `a*x=b` over a prime field, represented by
`v=(a,b)`, and a nonempty provenance set `T`. The row is available exactly when all
sources in `T` remain. Independent alternative support is represented by separate
rows, not by treating an OR dependency as an AND. Each `T` has at most `g` sources.
For every withdrawal set `D` with `|D|<=k`, the retained certificate must have the
same augmented row span as all surviving evidence. This preserves the solution
set, contradictions, and which linear questions have determined answers. It does
not infer a hypothesis class or extract equations from prose.

First consider `g=1`. Give each source a distinct field element `s`, and lift its
row to `v ⊗ (1,s,...,s^k)`. Retain the original rows whose lifted vectors extend a
linear basis. For a withdrawal set `D`, form `P_D(t)=product(t-d for d in D)`.
Contracting each lift with this polynomial's coefficients gives `P_D(s)*v`.
It is zero precisely on withdrawn sources and is a nonzero scalar multiple of
the original row otherwise. A linear map preserves a spanning relation, hence
the surviving retained rows span exactly the surviving original rows. With
original row rank `r`, at most `(k+1)*r` original rows are retained.

For joint provenance, let `u(s)=(1,s,...,s^(k+g-1))` and encode a provenance set by
the exterior product `w(T)=wedge(u(s) for s in T)`. Sets smaller than `g` are padded
with reserved, nonwithdrawable source values. Lift the equation as `v ⊗ w(T)` and
again select a basis of lifts while retaining the corresponding original rows.
There are `binomial(k+g,g)` exterior coordinates, so the retained-row count is at
most `r*binomial(k+g,g)`.

To verify the withdrawal property, extend `D` to size `k` with separate reserved
values and wedge against its source vectors. The result is a Vandermonde
determinant: it vanishes when `T` intersects `D` and is nonzero otherwise. Apply
this linear functional to the lift-spanning relation. The remaining coefficient
rows therefore span the same space as all remaining evidence. The field must
contain distinct values for every source and both kinds of reserved values; the
implementation validates that finite universe instead of silently hashing IDs
into collisions.

The bound is attained within the subset-certificate contract. For each of `r`
independent directions, include one copy with every `g`-subset of a fixed
`k+g`-source universe as its provenance. Deleting the complementary `k` sources
makes any chosen copy the sole surviving row in its direction. All
`r*binomial(k+g,g)` rows are necessary. This is not a lower bound on arbitrary
non-subset encodings. In particular, these highly regular worst cases may have
short descriptions if symbolic generation is allowed.

Certificate composition follows from the same equality: certificates for two
collections can be merged and compressed again. Their unions preserve every
allowed deletion before the second compression. This does not reset a consumed
withdrawal budget or permit indefinite deletion without additional evidence.

## Prior art and what remains research

[Fomin, Lokshtanov, Panolan and Saurabh](https://doi.org/10.1145/2886094) develop
efficient representative-family constructions using linear algebra and the
classical binomial-size bounds. The exterior encoding here belongs to that
established family of techniques; the tensor construction is not a newly invented
algebra. [Their product-family work](https://arxiv.org/abs/1402.3909) is another
relevant algorithmic control.

[Pintér and Varga](https://arxiv.org/abs/2302.11035) study color avoidance in graphs
and matroids, including approximation algorithms for single-color failures.
[Petruschka, Sapir and Tzalik](https://arxiv.org/abs/2311.08868) establish colored
fault-tolerant graph sparsification and also consider color lists. Source-coherent
withdrawal alone is therefore not a new fault model. The experiment includes an
exact single-withdrawal control using a primary basis and repairs in each source's
missing quotient space, retaining at most twice the original rank.

[Cherapanamjeri et al.](https://arxiv.org/abs/2506.13048) study space complexity of
learning and unlearning, including version-space compression and bounded deletion.
Their work reinforces why ordinary sample compression is insufficient here.
Reversible learning itself predates this project; this experiment supplies
equations rather than solving learning from natural-language episodes.

The useful research question is whether this exact algebra can become a practical
memory operation with realistic provenance, useful uncertainty and small repair
cost. A contribution would need evidence beyond applying existing mathematics to
a new name. This construction offers a mechanism to test, not a novelty claim.

## Frozen experiment

[PLAN.json](../../../benchmarks/research/source_lift/PLAN.json) was written before
the generated cases. The implementation is
[probe.py](../../../benchmarks/research/source_lift/probe.py). It uses standard-library
modular arithmetic, retains original row identities and provenance, and rejects
requests beyond the certified withdrawal budget. Six regression tests cover
polynomial annihilation, joint provenance, independently enumerated actual
solutions, contradictions, tight bounds, budget/input boundaries, strong repair
controls and merge composition.

The exhaustive cohort contains every triple of subspaces of a two-dimensional
space over GF(7), with single-source and pair-source provenance, at budgets zero,
one and two: 6,000 cases. The 30 tight constructions cover ranks 1/3, budgets
0–4 and provenance widths 1–3. Twelve larger cases use fixed seeds 3101–3103,
eleven latent variables, and redundant single-source evidence, joint-source
evidence, one rare direction, or a retractable contradiction. The candidate never
receives the generator's latent assignment.

Controls retain all original packed rows, a separate basis per provenance set,
and the union of bases for every allowed withdrawal. The last is expensive but
exact and can produce a smaller certificate than the lifted basis. The predefined
mechanism screen requires at most half the per-provenance bytes and at most 1.1
times the union-basis bytes on every seed of both redundant families. Compilation
time and peak traced allocation are recorded, with tracing enabled for every
compiler; shared-host timing cannot qualify superiority.

Raw run: `/Volumes/A/.hms-bench-cache/warm-source-lift-20261103-v1`.
All 6,000 exhaustive cases / 22,713 withdrawal states, 30 tight constructions, and
12 larger cases passed exact row-span comparison against independently reduced
original evidence. All tight cases attained the bound. Raw and source hashes were
verified after completion; source copies are retained with the raw evidence.
Freeze SHA256:
`9c02362ba29e4f3e8edaf0668ef5cd3acc86cd94a6dbf54ad27b0d8aaab56fa7`.
`source-lift-{freeze,finished}.json`, `source-lift-tests.log` and
`source-lift-run.log` retain receipts and logs here. No paid service or held-out
serving input was involved.

The predefined mechanism screen **failed**: only one of the six required redundant
cases passed both byte conditions. The exact construction works, but its lifted
linear basis preserves more structure than a smaller deletion-specific certificate
needs. The following byte ranges include all three seeds:

| Family | Original packet | Per-provenance basis | Exhaustive union basis | Lifted certificate |
|---|---:|---:|---:|---:|
| Redundant single-source | 11,917 | 4,849–5,004 | 850–943 | 1,036 |
| Redundant source-pairs | 12,685 | 10,408–11,266 | 1,168–1,432 | 2,191 |
| Rare direction | 11,948 | 4,508–4,694 | 850–912 | 974 |
| Retractable contradiction | 11,948 | 4,880–5,035 | 881–974 | 1,067 |

Thus the naive per-provenance control would overstate the candidate's value. The
union control is smaller in every larger case. Compilation and traced-allocation
figures remain in the raw receipts, but are not qualified timings; in particular,
the exhaustive control could further improve construction time by early stopping
once it spans the original rank. Do not advertise the observed traced timings as
an advantage over an optimized implementation.

The next algorithmic question is whether the valid algebraic certificate can act
as a bounded intermediate representation, followed by exact source-aware pruning
that tests only attainable withdrawals. The proven composition property permits
that investigation without rereading discarded evidence. Kernelization followed
by semantic minimization is itself established methodology; a useful result must
show a new practical tradeoff against optimized exact controls on newly frozen
inputs. The failed gate above is retained and will not be repaired by tuning its
seeds, thresholds or source order.

## Interpretation limits

Packed certificate bytes include retained equations, source IDs, original row IDs,
field, dimensions and the withdrawal contract. They exclude source text and the
full episode archive. Compilation retains temporary lifted bases and provenance
codes; those costs are measured separately. Neither small row counts nor traced
allocations establish whole-process memory savings.

This is a counterfactual answer-equivalence contract. It does not prove secure
erasure or that retained internal state has the same distribution as rebuilding
without withdrawn evidence. It does not preserve every original source citation,
only enough original evidence to preserve the specified equation semantics.
Temporal precedence, trust weighting, extraction errors, nonlinear reasoning and
indefinite mutations remain separate problems. The held-out throughput target
and the eight milestone criteria remain unchanged.
