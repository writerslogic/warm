# As-of recall drop: post-filter dilution test (lever/as-of-overfetch)

Status: protocol registered before any arm was run. Results and decision are appended below
the line "Outcome" in a later commit.

## Question

On the LongMemEval_S dev split the memory API loses recall when it is evaluated as of the
question date: temporal-reasoning recall_any@5 is 0.76 with `as_of` against 0.88 without
(`benchmarks/results/memory_api_longmemeval_dev.json`, 19 and 22 of 25 questions). Hypothesis H1
(from arXiv 2607.26520, abstract only verified: time-travel retrieval raised knowledge-update
R@10 but hurt temporal-reasoning R@10, attributed to filtering after top-k): the drop is
post-filter dilution, so fetching more candidates before the validity filter recovers it.
Hypothesis H2 (untested guess): closed validity windows hide facts that are still needed.

## What was read before registering

- `src/core/memory_api/recall.rs` `run_once`: when the store has any windowed or superseded fact
  and `as_of` is set, the eligible ids (status current at `as_of`) are computed first by a
  scan and passed to the search as `document_ids`. The filter runs BEFORE top-k, not after. The
  later per-hit `evaluate` only re-checks the already restricted set. So the memory API as
  shipped is not a post-filter design, and H1 as stated cannot describe it.
- `src/bin/research/memory-api-lme.rs` ingests each haystack session with `valid_from` = session
  date and no `valid_to`, no `supersedes`. The only filter that can fire is
  `valid_from > as_of`. H2 (closed windows) cannot be exercised on this data: there are none.
- Committed rows: knowledge-update `memory_as_of` 15/15 at @5 and @10 (no loss); temporal-reasoning
  19/25 at @5 and 21/25 at @10 against 22/25 and 24/25 without `as_of`. The per-question rows
  behind the gap were not inspected for this purpose before this protocol.
- The harness note already says some evidence sessions are dated after the question date.

## Arms

All on the 40 non-abstention knowledge-update (15) and temporal-reasoning (25) questions of the
dev ids in `benchmarks/public/longmemeval_split.json` (split seed 20261005). The held-out ids are
never selected (the binary refuses to run if one is). Lexical BM25 through the memory API, 1024
dimensions, session-level facts of user turns, as in `memory-api-lme`. No model, no LLM, no
network. The retrieval is deterministic, so there is no run seed; determinism is checked by a
repeat run that must be byte-identical.

| arm | definition |
|---|---|
| `no_as_of` | `Memory::recall`, no `as_of` (document search order) |
| `as_of` | `Memory::recall` with `as_of` = question date (current behavior, filter before top-k) |
| `post_x1` | `no_as_of` top k, then drop facts with `valid_from > question date`, keep the rest (filter after top-k, no overfetch) |
| `post_x2`, `post_x4`, `post_x8` | `no_as_of` top (f * k), drop facts with `valid_from > question date`, keep the first k; f = 2, 4, 8 |

k is the evaluated cut-off (5 or 10), so the pool for @5 is 5f candidates and for @10 is 10f. The
`post_*` arms emulate the preprint's filter-after-top-k design inside the harness through the
public API (`valid_from` is returned on every hit). `post_x1` is a reference arm added so that
dilution, if it exists, is visible at all; it is not one of the overfetch candidates.

Metrics: recall_any@5 and @10 per question type (hits out of 15 and 25), as in the repo.

Diagnostics (no tuning, computed from the data):

- D1 ceiling: for each question, the number of evidence sessions with `valid_from <= question
  date`. A question with zero such sessions cannot be answered by any filter-respecting retrieval.
- D2 gap decomposition: among questions that `no_as_of` hits at @5 and `as_of` misses at @5, how many
  have zero valid evidence (unrecoverable by retrieval) and how many have at least one (recoverable
  in principle).
- D3 data facts for H2: count of facts with `valid_to` and of supersession edges (expected 0 and 0).

Paired counts (wins, losses, ties against `as_of`, and against `no_as_of`) with exact two-sided
sign-test p are reported for every arm, metric and question type.

## Decision rule (fixed now)

Let G = hits(`no_as_of`) - hits(`as_of`) on temporal-reasoning at @5 (expected 3).

Ship an opt-in `overfetch` option only if, for at least one f in {2, 4, 8}, all hold:

1. temporal-reasoning recall_any@5 of the `post_xf` arm is at least ceil(G / 2) questions above
   `as_of` (recovers at least half of the gap);
2. knowledge-update recall_any@5 and @10 of that arm lose nothing against `as_of`;
3. temporal-reasoning recall_any@10 of that arm is not below `as_of`.

Otherwise no-go: code unchanged, the no-go and the mechanism recorded here. With 25 questions a
gain of 2 or 3 questions has two-sided sign-test p of 0.5 or 0.25 at best, so a go is provisional
and licenses one held-out run by the maintainer, not a claim. An `overfetch` option, if shipped,
defaults to off and the default path must stay bit-identical (tested).

## Expected outcomes (registered, may be wrong)

1. `post_x1` is below `as_of` (dilution exists in the emulated design). `post_x2` to `post_x8`
   rise monotonically towards `as_of` and never exceed it: with a score that does not depend on
   the candidate set, filter-before-top-k is the limit of filter-after-top-k with unbounded
   overfetch.
2. D2 shows that most or all of the gap questions have zero valid evidence sessions: the drop is
   the contract working (evidence dated after the question date does not exist as of that date),
   not dilution.
3. D3 is 0 and 0, so H2 is not testable on LongMemEval; it stays an untested guess.
4. The rule returns no-go.

## Not tested

- Dense retrieval with the cached all-MiniLM-L6-v2 session embeddings
  (`~/.cache/hms-bench/longmemeval_s`). The memory API has no dense path in this harness; the dense
  `as_of` cost (1 to 3 questions of 40 at @5) is reported in `docs/research/fpe-timestamps-164.md`
  on the `issue/164-fpe-timestamps` branch, from its own scorer. A dense post-filter arm would need
  that scorer.
- The held-out split, LongMemEval_M, any reader or judge, answer accuracy.
- Real `valid_to` or `supersedes` data (H2).
- Whether the preprint's finding holds for any other system.
