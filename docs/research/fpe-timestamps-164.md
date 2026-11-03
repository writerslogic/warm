# Fractional power encoding of timestamps (issue #164)

Status: protocol fixed before the run. Results are appended below by the run commit.

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
