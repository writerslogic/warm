# Preserving changed answers when old facts can return

The source-span experiments concern a monotone evidence language: removing
equations cannot add a linear consequence. Real memory has a different failure
mode. Withdrawing a newer assertion can expose an older assertion, so an answer
can become supported after evidence is removed. A certificate that records only
the current positive proofs is insufficient for that contract.

For example, source 1 says a claim holds; source 2 later says it does not. A
separate source supports a contrary claim. Under the declared latest-eligible
policy, the first claim is initially unsupported. Withdrawing source 2 revives
it, leaving both claims supported. Retaining only initially positive evidence
misses this change. This is an experimental semantics example, not a report of
a shipping WARM defect: WARM defaults to explicit supersession and exposes a
separate `LatestBySubject` policy. The experiment changes no serving behavior.

## A certificate for the executed decision

The prototype in
[temporal_trace](../../../benchmarks/research/temporal_trace/PLAN.json) evaluates
bounded deterministic Boolean programs over temporal slots. Records carry exact
source dependencies, original text, valid intervals and recorded-at times. A
record is available only if none of its sources has been withdrawn. At a declared
valid/known-time snapshot, a slot selects its eligible record with maximum
`(valid_from, recorded_at, ordinal)`. Valid intervals are half-open. Independent
positive and negative claim slots can both hold; a contradiction does not imply
unrelated facts. Input observations and programs are supplied, not extracted
from natural language.

The execution oracle records **every slot read**, including a false comparison
or branch condition. It retains the selected original records and recursively
withdraws sources appearing in those records, up to a fixed budget. Empty reads
need no source branch. The certificate is the union over visited executions and
declared snapshots. Its reader has no access to the original collection and
refuses undeclared snapshots and over-budget withdrawals.

Why it works: fix a final withdrawal set `F`. At a visited set `D` contained in
`F`, either `F` hits a source in the current read trace, in which case the search
includes that next branch, or every selected record survives `F`. In the latter
case each selected record remains the full collection's head: deleting other
records cannot displace a surviving maximum. An empty slot stays empty under
further deletion. Induction over deterministic execution then preserves every
read and branch, including short circuits. The certificate contains those heads;
as a subset of the original input, it cannot introduce a newer eligible record.
It therefore reproduces the same trace and result. Each recursive step adds a
member of `F`, so the search reaches this case within budget.

The proof also gives exact equality with exhaustive trace union. Every visited
state is admissible, so no selected record lies outside that union. Conversely,
every admissible trace is reproduced at a visited state, so none is missing.
If a trace touches at most `w` source identities, the un-memoized search has at
most `sum(w^i, i=0..k)` nodes per snapshot. This can still be expensive or retain
nearly all evidence. Source identities shared across records must stay shared;
independent-record deletion is a different problem.

This generalizes the tested oracle beyond linear algebra by using a program's
control dependencies. It preserves a fixed finite query/snapshot contract and
original read provenance, not arbitrary future questions, unlimited corrections,
all derivations, or source extraction quality. New evidence, changed programs,
new snapshots or a renewed withdrawal budget require recertification against
adequate original state. The certificate alone cannot silently renew its budget.

## Controls and frozen test

The first five regression tests pass, including the negative-control witness,
valid/known-time separation, half-open boundaries, joint provenance, contradictory
claims, 80 randomized histories at three withdrawal budgets, exact trace-union
agreement and reader/input boundaries. See `temporal-trace-tests.log`.

The frozen experiment enumerates all three-valued observations and source
assignments for two two-revision slots, then runs 32 seeded histories and 12
larger cases. Each case compares the candidate with:

- an indexed history restricted to statically reachable slots and snapshots;
- a stronger per-slot head certificate, constructed by branching only on the
  current head's sources;
- exhaustive trace union as a correctness oracle, not a weak timing competitor;
- positive-record-only branching as an intentionally unsound negative control.

The size screen requires at least 30% fewer serialized bytes than the smaller
positive temporal control in every independent/joint-guard case. Unconditional
queries are a mandatory stress regime. Complete text, source identities,
programs, times and budget metadata are charged to every packet. These bytes are
neither model tokens nor whole-process memory. Construction and evaluation run
locally with no paid services and no held-out serving data.

Run: `/Volumes/A/.hms-bench-cache/warm-temporal-trace-20261103-v1`.
The experiment completed **3,932 cases / 86,846 snapshot-withdrawal checks** with
zero output, trace or original-record mismatches. In every case the recursive
packet was byte-identical to exhaustive trace union. The positive-only negative
control changed the actual Boolean answers in 45,725 checks, in addition to
provenance failures. All 82 recorded source/raw hashes were verified afterwards.
Freeze SHA256:
`62c164450d0b0fd1ee55eac6d5957a4f2acc00bc7671ff915155092576f03736`.
Receipts are `temporal-trace-{freeze,finished}.json`; the run log is
`temporal-trace-run.log`.

The frozen size screen **failed: four of eight required cells passed**.

| Case family | Candidate records | Strong temporal control records | Packet-byte reduction |
|---|---:|---:|---:|
| Independent guard sources | 14–15 | 150–157 | 84.09–84.49% |
| A shared dependency across guards | 130–145 | 150–157 | 7.16–13.43% |
| Unconditional reads | 150–157 | 150–157 | 0% |

The same cohort seeds, values, snapshots and acceptance rule were retained. In
the independent family, no two withdrawals can invalidate all three guards, so
the alternative branch is unreachable within the contract. Giving those guards
one shared source makes that branch reachable after just one withdrawal. Its
previously irrelevant histories then become necessary. This explains the
measured loss of compression; it is not a compiler correctness failure.

The tested program's source dependency structure therefore matters more than
its initial apparent redundancy. Small withdrawal count alone does not imply
a small certificate: a single shared dependency can expose arbitrarily many
independent answers. This inference follows from the explicit branch construction,
not from an empirical impossibility claim about general memory. Requiring exact
read identities also deliberately charges evidence that an answer-only interface
might not need.

These comparisons do not establish a new best compiler. A query-specialized
symbolic compiler can discover the same unreachable branches, as the completed
control below confirms. The unconditional
cases also expose avoidable construction work: the trace compiler used up to
2,528 oracle calls where the per-slot control used at most 531 over the cohort.
No timing or whole-memory superiority is claimed, and no second parameter sweep
was used to rescue the failed screen.

## Stronger symbolic control

[symbolic_control.py](../../../benchmarks/research/temporal_trace/symbolic_control.py)
uses established [ordered binary decision diagrams](https://www.cs.cmu.edu/~bryant/pubdir/ieeetc86.pdf).
A variable represents withdrawing one source. A record's availability is the
conjunction of its surviving sources; its head condition additionally excludes
every newer eligible record. Program branches propagate Boolean reachability to
each slot read. A shortest accepting path, charging one for each withdrawn source,
decides whether a particular head can be read within budget. Source order is fixed
by ascending identity, and node/cache limits reject oversized compilations.

This is an independent symbolic construction of the same contract. Four new
tests check Boolean truth tables and minimum-withdrawal costs, 90 randomized
temporal/budget cases against exhaustive execution, the shared-source witness and
allocation limits. All nine combined tests pass; see `temporal-symbolic-tests.log`.

The subsequent frozen audit at
`/Volumes/A/.hms-bench-cache/warm-temporal-symbolic-control-20261103-v1` ran both
compilers in three alternating repetitions on the unchanged 12 exposed cases.
**Every symbolic packet was byte-identical to the trace compiler's packet.** The
prior cohort's 52,464 snapshot-withdrawal checks therefore transfer exactly.
All 20 source/raw hashes were verified. Freeze SHA256:
`471a75e3dab0bcd707e6ce9a2473b97604868b5b59c55945e0499719670c587d`.
Receipts and log are `temporal-symbolic-{freeze,finished}.json` and
`temporal-symbolic-run.log`.

The symbolic control was also faster in every development cell: its median
construction time was 0.044–0.666 times the trace compiler's. These sub-millisecond
to tens-of-milliseconds shared-host measurements are not qualified system ratios.
The compiler used 155–2,776 decision nodes, with no source-order tuning. The earlier
84% independent-source packet reduction consequently supplies no advantage over
ordinary query-specialized compilation. Retaining exact trace identities also
fixes the minimum permissible record union, so any correct original-record
certificate for this contract must include it.

The completed experiment establishes the semantic requirement and the failure-
correlation tradeoff, while rejecting the proposed implementation as a distinctive
advance over these controls. It should not be promoted or renamed as an invention.

## Prior art and the unresolved contribution

Control dependencies are established in
[dynamic program slicing](https://jstolarek.github.io/files/ricciotti_stolarek_perera_cheney_icfp2017.pdf)
and [dependency provenance](https://www.cambridge.org/core/journals/mathematical-structures-in-computer-science/article/abs/provenance-as-dependency-analysis/1C7F70EF51CFC2045ADB2F50615F7A6C).
[Why-not provenance](https://arxiv.org/abs/1701.05699) explicitly handles failed
derivations and negation. [Incremental view maintenance](https://doi.org/10.1145/170036.170066)
already supports deletion and rederivation. The experiment does not claim to
invent any of these mechanisms.

The research question is whether bounding the allowed future mutations makes
exact, provenance-bearing decision memory useful at materially lower cost.
A successful packet screen would justify comparison with query-specialized
symbolic compilation, incremental provenance and real reader workloads. The first
of those stronger comparisons is now complete and rejects the algorithm advantage.
Even a future successful comparison would
not establish priority, a model answer-quality advantage, or milestone 6. The
preselected system-throughput gate remains unchanged.
