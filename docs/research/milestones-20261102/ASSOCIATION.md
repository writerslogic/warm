# Guarded tensor association: rejected development probe

The frozen synthetic probe preserved every tested winner but failed its promotion
gate after correcting an unfair control layout. It does not establish a new
memory mechanism, competitive advantage, memory saving, or milestone completion.
No native implementation is justified by this experiment.

## Question and scope

The [plan](../../../benchmarks/research/certified_association/PLAN.json) tests a
continuous association operator: two normalized, nonnegative cue vectors contract
a 128 × 128 × 64 weight tensor, and the largest output wins. Rank-four tensor
trains inferred from observed input supply approximate scores; explicit sparse
corrections preserve large residuals. A residual-plus-roundoff margin check may
accept the winner; otherwise the original tensor is evaluated. Tensor trains,
sparse corrections and margin bounds are established methods, not inventions here.

Three seeds each generate correlated, rare-exception and independent weights.
Every case has 64 queries at each of supports 4, 64 and 128, with three alternating
control/candidate timing repetitions. Exact controls use indexed gather for small
supports and dense two-stage BLAS otherwise. All input, factor, correction and
fallback costs remain visible. No generator factors reach the candidate.

The frozen gate requires both median and p95 at least 2× faster in **every**
correlated/rare-exception seed at supports 64 and 128, at least 90% certification,
and zero wrong winners throughout. It licenses a native experiment only. The
retained original prevents any claim of reduced total storage.

This operator experiment is narrower than the fact-storage screen proposed in
RESEARCH.md. It does not test discrete fact membership, provenance, temporal
conflicts, context tokens, or an LLM. It does not discharge that screen's memory
gate or replace its exact sparse/bitmap controls.

## Validity correction

Version 1's structured generator returned Fortran-contiguous arrays with strides
`[8, 1024, 131072]`. Its control's reshape copied the full 8 MiB tensor on each
query, whereas the candidate's retained copy was C-contiguous. Its timing-based
promotion result is invalid and must not be cited as a speedup.

Version 2 makes the shared generated input C-contiguous once, outside timing,
and checks that the control's reshape shares memory. The test suite now covers
that invariant for all three generators. All nine original and mutated input
hashes match version 1. Seeds, query generation, algorithms, thresholds and the
promotion gate are unchanged. The original receipts remain intact.

## Corrected measurements

Ratios below are control/candidate time; greater than one favors the candidate.
Ranges retain every seed in the stated family. Timings are exploratory on a
shared Apple M4 host, not isolated serving benchmarks.

| Scenario | Support | Median ratio | p95 ratio | Certified queries | Target cells passing |
|---|---:|---:|---:|---:|---:|
| Correlated | 64 | 1.095–1.199× | 1.061–1.422× | 192/192 | 0/3 |
| Correlated | 128 | 1.061–1.202× | 1.062–1.300× | 192/192 | 0/3 |
| Rare exceptions | 64 | 0.835–0.953× | 0.899–0.951× | 192/192 | 0/3 |
| Rare exceptions | 128 | 0.840–0.931× | 0.837–0.961× | 192/192 | 0/3 |
| Independent | 64 | 0.931–0.985× | 0.952–0.975× | 0/192 | Not a promotion target |
| Independent | 128 | 0.937–0.977× | 0.938–1.042× | 0/192 | Not a promotion target |
| All scenarios | 4 | 0.803–0.845× | 0.805–0.860× | 0/576 | Not a promotion target |

All 1,728 distinct initial queries (5,184 measured repetitions) and 144 mutation
queries returned the exact control's winner. The independent case disabled the
approximation because too many corrections were required. Numerical certification
uses a conservative double-precision error model; these observations are not a
formal machine-checked floating-point proof for arbitrary tensors.

Each case retains 8,388,608 original bytes and 22,528 factor bytes. Correlated
cases need no corrections; rare-exception cases need 20,960 correction bytes;
independent cases retain 18,845,640–18,857,000 correction bytes even though their
fast path is disabled. This is a storage loss. Build times were
0.398–0.437 seconds and full mutation rebuilds 0.396–0.456 seconds. Whole-process
peak RSS was 245,284,864 bytes, including co-resident comparison objects and
scratch; it is not a per-arm memory comparison.

Mutation means overwriting one weight and zeroing another, then fully rebuilding.
Every rebuilt state matched a separate fresh rebuild. Zero is still a valid
association score, and the experiment retains old comparison objects. This is
representational rebuild equivalence, not deleted-record semantics or secure
memory erasure.

## Interpretation and reproduction

The measured failure rules out promoting this Python implementation under this
protocol. It does not prove that every native tensor operator is slow. More
fundamentally, raw fallback retains the original storage cost, and continuous
argmax agreement does not show preservation of rare factual evidence. A useful
next research question must address those constraints before another port or
parameter sweep.

Five focused tests pass, including an independent scalar contraction, exact ties,
small margins, mutation, invalid inputs and the control-layout regression.
Source, locked dependencies, pre-run hashes, every query/cue/timing and complete
results are retained at:

- Invalid timing attempt: `/Volumes/A/.hms-bench-cache/warm-certified-association-20261103-v1`.
- Corrected attempt: `/Volumes/A/.hms-bench-cache/warm-certified-association-20261103-v2`.
- Repository receipts: `association-v2-freeze.json`, `association-v2-finished.json`.
- Test/run logs: `association-tests-v2.log`, `association-run-v2.log`.

Reproduce from the repository root with a new output directory:

```sh
OPENBLAS_NUM_THREADS=2 OMP_NUM_THREADS=2 VECLIB_MAXIMUM_THREADS=2 \
uv run --project benchmarks/research/certified_association --frozen python \
benchmarks/research/certified_association/probe.py --output /absolute/new/output
```
