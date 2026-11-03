# Fractional power encoding of timestamps (issue #164)

Status: complete. Protocol and decision rule fixed before the run; results and decision below.

## Question

The memory API applies `valid_from`, `valid_to` and `supersedes` as filters
(`src/core/memory_api/recall.rs`, status resolution: superseded if a superseder's effective
time is `<= now`, expired if `valid_to <= now`, not yet valid if `valid_from > now`), plus an
optional multiplicative recency policy (`ExponentialRecency`). Does encoding timestamps with
fractional power encoding (FPE), so that recency and validity take part in similarity, retrieve
the evidence session more often than that filter approach? Approximate nearest-neighbor
integration is out of scope.

## Reference check

The issue cites "NeCo, arXiv 2609.38471". The abstract at that ID (fetched 2026-11-03) is
"Derandomizing Dense Binary Hypervector Codebooks for Quantized Scalars" (Rachkovskij, Osipov,
Volkov, Kleyko, Snasel): transition-based construction of dense binary level codebooks whose
similarity follows an exponential or linear law in the level separation. The abstract does not
mention fractional power encoding or the name NeCo. The prototype therefore follows the standard
FPE construction (a random phasor base raised to the fractional power `t`; the inner product is a
shift-invariant kernel in `t`), not that paper's binary construction. The binary construction is
a candidate for a quantized, binary-substrate follow-up and was not built.

## Method

Prototype: `src/core/experimental/fpe_time.rs` (feature `experimental`).

- `z(t)_j = exp(i theta_j t)`; `dot(z(a), z(b)) = mean_j cos(theta_j (a - b))` estimates a kernel.
  Half-Cauchy frequencies give `exp(-|d|/sigma)`; Gaussian frequencies give a Gaussian. Frequencies
  are a stratified quantile grid with seeded jitter (SplitMix64), so `(kernel, sigma, dim, seed)`
  fixes the encoder.
- Validity window `[a, b)`: the integral of `z(s)` over the window, normalized by the kernel mass.
  Its inner product with `z(t)` is the kernel mass inside the window, a soft membership in
  `[0, 1]`. Edges are blurred over about `sigma`.
- `FpeRecency` implements the memory API `RecencyPolicy` with factor
  `(1 - w) + w * max(sim(now, event_time), 0)`.

Harness: `src/bin/research/fpe-timestamps-eval.rs` (features `experimental`, `cli`). Retrieval only:
no LLM, reader or judge, no network.

Data: LongMemEval_S, only the 100 ids listed as `dev` in `benchmarks/public/longmemeval_split.json`
(split seed 20261005), abstention questions excluded. Temporal slice = knowledge-update plus
temporal-reasoning dev questions; the remaining dev questions are a no-harm check. The held-out
ids are never selected (the binary refuses to run if a held-out id is selected). Dense path: the
cached all-MiniLM-L6-v2 session embeddings of the benchmark harness (no embedding run). Lexical
path: the memory API's BM25 recall as in `memory-api-lme`, with the FPE policy plugged in through
`Memory::with_recency`; its existing rows are compared with the committed
`benchmarks/results/memory_api_longmemeval_dev.json`.

Metric: recall_any@k (the repo's retrieval metric) at k = 1, 3, 5, 10, with recall_all@5/10 and
newest-evidence@5/10 as secondary. Score = max(cosine, 0) times a time factor in `[0, 1]`;
`as_of` = question date; session date = haystack date.

Configurations:

- existing family: plain; `as_of` filter; `as_of` filter plus exponential decay with half-life
  7, 30, 90, 365 days;
- FPE family: variant (point kernel to the question date; validity window `[session date, evaluation time + 6 sigma)`; relative-time target parsed from the question, else window) x sigma 7, 30, 90, 365
  days x mixing weight 0.25, 0.5, 1.0 x with or without the hard `as_of` filter x vector encoding
  or exact closed-form Laplace kernel;
- oracle supersession (secondary, labels not available at serve time): the older evidence
  sessions of a question are superseded by its newest one; hard rule = filter them once the
  newest is effective, FPE rule = truncate their window at the newest date.

Tuning: one grid, dimension 1024, seed 164, on the dev slice only. Selection rule for both
families: maximize recall_any@5 on the temporal slice, then @10, then recall_all@5; first in
enumeration order on a tie. The FPE family has 12x more configurations than the existing one, so
selection favors FPE; a no-go under that bias is therefore the stronger result.

## Decision rule (fixed before the run)

Go requires all of:

- R1: the selected FPE config beats the selected existing config by net >= 2 questions (paired
  wins minus losses) on recall_any@5 or on recall_any@10 over the temporal slice;
- R2: net >= 0 against plain on recall_any@5 over the other dev questions (no harm);
- R3: net >= 1 against the selected existing config at recall_any@5 on the temporal slice for at
  least 5 of the 6 seeds (164 and 1 to 5) at dimension 1024.

Anything else is no-go. A go is provisional: it licenses one held-out run by the maintainer, not
a ship decision. The write-up also reports whether the exact closed-form kernel matches the
vector encoding, which separates "the kernel shape helps" from "the vector representation helps".

## Not tested

- The held-out split (400 ids). Nothing here is a held-out claim; a go would be provisional.
- LongMemEval_M, any LLM reader or judge, answer accuracy. Retrieval metrics only.
- LoCoMo and the other `benchmarks/results/` memory sets (`locomo_v1.json`, `locomo_v2.json`):
  they report LLM-judged accuracy with their own tuning and held-out protocol, and their only
  embeddings came from an API model, so no cached local embeddings exist and the rules here forbid
  an API run. The only committed memory-API retrieval set with timestamps is
  `memory_api_longmemeval_dev.json`, which the lexical path reproduces and extends.
- Approximate nearest-neighbor integration (out of scope): the case where time must live inside
  the vector so that an index can use it. Here the time factor multiplies a score after retrieval,
  which a scalar decay already does.
- Real supersession links: LongMemEval carries no subject or `supersedes` labels, so the memory
  API's supersession path is not exercised; only the oracle edges derived from the evidence labels
  are (secondary rows).
- Gaussian kernels (implemented and unit-tested, not in the grid), the Cauchy-quantile variance
  across more than 6 seeds, dimensions other than 64, 256, 1024, 4096, other embedding models,
  non-UTC or sub-day timestamps, `valid_to` windows in the data (LongMemEval has none).
- A half-line window code for open-ended validity (not built). The closed code is accurate only
  while `(end - start) * (pi / 2) / (sigma * dim)` is well below 1; an earlier horizon of 100
  years put the vector rows at noise of order 0.4 and was replaced by `evaluation time + 6 sigma`
  before any result was recorded.
- The relative-time parser handles "N days/weeks/months/years ago", "last week/month/year" and
  "yesterday" only.

## Run notes

Command: `fpe-timestamps-eval --data ~/.cache/hms-bench/downloads/longmemeval_s_cleaned.json
--split benchmarks/public/longmemeval_split.json --embeddings ~/.cache/hms-bench/longmemeval_s
--committed benchmarks/results/memory_api_longmemeval_dev.json --out
benchmarks/results/fpe_timestamps_164_dev.json` (debug build, seed 164). Result file:
`benchmarks/results/fpe_timestamps_164_dev.json` (all 288 grid rows, oracle rows, dimension and
seed sweep, lexical rows). The dataset checksum equals the cache metadata
(d6f21ea9d60a...). An earlier invocation aborted while loading data (the cache renames evidence
sessions `answer_*` to `noans_*`, a lookup fix committed afterwards) and recorded nothing, so the
protocol and decision rule above predate every number below. The lexical path matches the
committed `memory_api_longmemeval_dev.json` in 13 of 14 shared rows; the exception is
knowledge-update `memory_recency_7d` (13/14 hits at k=5 here against 14/14 committed), not
investigated.

## Results (seed 164, dimension 1024, dev split only)

Temporal slice: 40 questions (knowledge-update 15, temporal-reasoning 25); other dev: 54. Relative-time targets parsed in 6 of 40 temporal questions.

Selected on the temporal slice: best existing `dense_plain`, best FPE `fpe_point_s7d_w0.5`, exact-kernel twin `fpe_point_exact_s7d_w0.5`.

| config | slice | recall_any@1 | @3 | @5 | @10 | recall_all@5 | newest@5 |
|---|---|---|---|---|---|---|---|
| `dense_plain` | temporal | 77.5% | 90% | 92.5% | 97.5% | 77.5% | 85% |
| `dense_plain` | knowledge_update | 93.3% | 100% | 100% | 100% | 86.7% | 93.3% |
| `dense_plain` | temporal_reasoning | 68% | 84% | 88% | 96% | 72% | 80% |
| `dense_plain` | other_dev | 87% | 92.6% | 96.3% | 98.1% | 88.9% | 90.7% |
| `dense_plain` | temporal | 77.5% | 90% | 92.5% | 97.5% | 77.5% | 85% |
| `dense_plain` | knowledge_update | 93.3% | 100% | 100% | 100% | 86.7% | 93.3% |
| `dense_plain` | temporal_reasoning | 68% | 84% | 88% | 96% | 72% | 80% |
| `dense_plain` | other_dev | 87% | 92.6% | 96.3% | 98.1% | 88.9% | 90.7% |
| `fpe_point_s7d_w0.5` | temporal | 72.5% | 90% | 97.5% | 97.5% | 80% | 92.5% |
| `fpe_point_s7d_w0.5` | knowledge_update | 80% | 100% | 100% | 100% | 80% | 93.3% |
| `fpe_point_s7d_w0.5` | temporal_reasoning | 68% | 84% | 96% | 96% | 80% | 92% |
| `fpe_point_s7d_w0.5` | other_dev | 79.6% | 94.4% | 98.1% | 98.1% | 88.9% | 92.6% |
| `fpe_point_exact_s7d_w0.5` | temporal | 72.5% | 90% | 97.5% | 97.5% | 80% | 92.5% |
| `fpe_point_exact_s7d_w0.5` | knowledge_update | 80% | 100% | 100% | 100% | 80% | 93.3% |
| `fpe_point_exact_s7d_w0.5` | temporal_reasoning | 68% | 84% | 96% | 96% | 80% | 92% |
| `fpe_point_exact_s7d_w0.5` | other_dev | 79.6% | 94.4% | 98.1% | 98.1% | 90.7% | 92.6% |

Paired comparisons (wins/losses/ties of A against B; net = wins - losses; exact sign-test p):

| comparison | A | B | @5 W/L/T (net, p) | @10 W/L/T (net, p) |
|---|---|---|---|---|
| fpe_vs_plain_temporal | `fpe_point_s7d_w0.5` | `dense_plain` | 2/0/38 (2, 0.5) | 0/0/40 (0, 1) |
| fpe_vs_best_existing_temporal | `fpe_point_s7d_w0.5` | `dense_plain` | 2/0/38 (2, 0.5) | 0/0/40 (0, 1) |
| fpe_vs_exact_twin_temporal | `fpe_point_s7d_w0.5` | `fpe_point_exact_s7d_w0.5` | 0/0/40 (0, 1) | 0/0/40 (0, 1) |
| best_existing_vs_plain_temporal | `dense_plain` | `dense_plain` | 0/0/40 (0, 1) | 0/0/40 (0, 1) |
| fpe_vs_plain_other_dev | `fpe_point_s7d_w0.5` | `dense_plain` | 1/0/53 (1, 1) | 0/0/54 (0, 1) |
| best_existing_vs_plain_other_dev | `dense_plain` | `dense_plain` | 0/0/54 (0, 1) | 0/0/54 (0, 1) |

Decision rule (from the result file): R1 true, R2 true, R3 true; net at @5 per seed (164, 1..5): [2,2,2,2,2,2]. Decision: **go**.

Selected FPE config across dimensions and seeds (temporal slice, recall_any@5/@10, net at @5 against the selected existing config):

| dim | seed | @5 | @10 | net@5 |
|---|---|---|---|---|
| 64 | 164 | 97.5% | 97.5% | 2 |
| 64 | 1 | 97.5% | 97.5% | 2 |
| 64 | 2 | 97.5% | 97.5% | 2 |
| 64 | 3 | 97.5% | 97.5% | 2 |
| 64 | 4 | 97.5% | 97.5% | 2 |
| 64 | 5 | 97.5% | 97.5% | 2 |
| 256 | 164 | 97.5% | 97.5% | 2 |
| 256 | 1 | 97.5% | 97.5% | 2 |
| 256 | 2 | 97.5% | 97.5% | 2 |
| 256 | 3 | 97.5% | 97.5% | 2 |
| 256 | 4 | 97.5% | 97.5% | 2 |
| 256 | 5 | 97.5% | 97.5% | 2 |
| 1024 | 164 | 97.5% | 97.5% | 2 |
| 1024 | 1 | 97.5% | 97.5% | 2 |
| 1024 | 2 | 97.5% | 97.5% | 2 |
| 1024 | 3 | 97.5% | 97.5% | 2 |
| 1024 | 4 | 97.5% | 97.5% | 2 |
| 1024 | 5 | 97.5% | 97.5% | 2 |
| 4096 | 164 | 97.5% | 97.5% | 2 |
| 4096 | 1 | 97.5% | 97.5% | 2 |
| 4096 | 2 | 97.5% | 97.5% | 2 |
| 4096 | 3 | 97.5% | 97.5% | 2 |
| 4096 | 4 | 97.5% | 97.5% | 2 |
| 4096 | 5 | 97.5% | 97.5% | 2 |

Lexical (BM25 through the memory API; recall_any, 40 temporal dev questions). Rows that exist in the committed file match it in 13 of 14 cases.

| question type | config | @5 | @10 |
|---|---|---|---|
| knowledge-update | `document_search` | 100% | 100% |
| knowledge-update | `memory_as_of` | 100% | 100% |
| knowledge-update | `memory_fpe_s30d_w0.5` | 100% | 100% |
| knowledge-update | `memory_fpe_s90d_w0.5` | 100% | 100% |
| knowledge-update | `memory_recency_30d` | 93.3% | 93.3% |
| knowledge-update | `memory_recency_365d` | 100% | 100% |
| knowledge-update | `memory_recency_7d` | 86.7% | 93.3% |
| knowledge-update | `memory_recency_90d` | 93.3% | 100% |
| temporal-reasoning | `document_search` | 88% | 96% |
| temporal-reasoning | `memory_as_of` | 76% | 84% |
| temporal-reasoning | `memory_fpe_s30d_w0.5` | 72% | 88% |
| temporal-reasoning | `memory_fpe_s90d_w0.5` | 76% | 84% |
| temporal-reasoning | `memory_recency_30d` | 72% | 88% |
| temporal-reasoning | `memory_recency_365d` | 76% | 84% |
| temporal-reasoning | `memory_recency_7d` | 80% | 88% |
| temporal-reasoning | `memory_recency_90d` | 72% | 84% |

## Reading of the numbers

- Rule output: R1, R2 and R3 are all true, so the pre-registered rule returns **go**, but only
  provisionally. The selected FPE config (`fpe_point_s7d_w0.5`: point kernel to the question date,
  sigma 7 days, weight 0.5, no hard filter) gains 2 of 40 temporal questions at k=5 (37 to 39 hits;
  net +2, 2 wins, 0 losses, exact sign test p = 0.5) and nothing at k=10 (39 of 40 both). It was
  selected from 72 vector configurations on those same 40 questions; the existing family has 6.
- The gain is not an FPE gain. The exact closed-form twin (`fpe_point_exact_s7d_w0.5`, no vectors)
  ties the vector config question for question (0 wins, 0 losses, 40 ties), and the result is
  identical at dimensions 64, 256, 1024, 4096 and six seeds. What helps is a symmetric proximity
  factor around the question date, not the vector encoding. It does not depend on the relative-time
  parser (the target variant is not the selected config).
- The best existing config on the temporal slice is plain dense retrieval (37 hits at k=5): the
  `as_of` filter and `as_of` plus exponential decay all score lower (34 to 36), because some
  evidence sessions are dated after the question date and the hard filter removes them
  (the same effect as in the committed lexical result).
- Soft validity windows (`fpe_window_*`) match plain (37 at k=5) or add one hit at k=10
  (`fpe_window_s30d_w0.5`: 37 and 40). The relative-time target variant reaches 38 at k=5 on 6
  parsed questions.
- Lexical path: no FPE policy config beats plain `document_search` on either question type
  (knowledge-update 100% for plain; temporal-reasoning 88% at k=5 for plain against 72 to 76% for
  the FPE policies, which run behind the `as_of` filter that causes the loss).
- Open-ended windows beyond the resolution limit alias the low frequencies (see Not tested).

## Decision

No-go on fractional power encoding of timestamps as a vector representation for the memory API.
The pre-registered rule returns a provisional go on a 2-question, k=5-only margin, which is within
selection noise (p = 0.5 from 72 configs on 40 questions), is reproduced exactly by a scalar
closed-form kernel, and is absent on the lexical path. FPE's distinct capability, time living
inside a vector that an index can use, was out of scope and is untested, so this is not evidence
against it for ANN integration. What may deserve one held-out run by the maintainer is the
scalar symmetric proximity factor (not FPE), and the finding that the hard `as_of` filter costs
1 to 3 questions of 40 at k=5 on dense retrieval here; neither is an FPE result.
