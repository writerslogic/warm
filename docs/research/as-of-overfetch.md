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

---

## Outcome

Result file: `benchmarks/results/as_of_overfetch_dev.json` (arm rows, paired sign tests, D1 to D3,
per-question hits). Command, from harness commit 95a1228d on base ae3fc4cb (debug build):

```
memory-api-lme --data ~/.cache/hms-bench/downloads/longmemeval_s_cleaned.json \
  --split benchmarks/public/longmemeval_split.json --out as_of_overfetch_dev.json --overfetch 1,2,4,8
```

Dataset sha256 `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442` (equals the cache
metadata cited in `fpe-timestamps-164.md`). Split seed 20261005, dev ids only, 40 questions, no
run seed (deterministic). Two runs were byte-identical (sha256 prefix `47b8df94a0b6a4b3`); the
shared rows equal the earlier base and main runs of Part 1.

recall_any hits at @5 / @10 (knowledge-update n = 15, temporal-reasoning n = 25):

| arm | knowledge-update | temporal-reasoning |
|---|---|---|
| `no_as_of` | 15 / 15 | 22 / 24 |
| `as_of` (current) | 15 / 15 | 19 / 21 |
| `post_x1` | 15 / 15 | 19 / 21 |
| `post_x2` | 15 / 15 | 19 / 21 |
| `post_x4` | 15 / 15 | 19 / 21 |
| `post_x8` | 15 / 15 | 19 / 21 |

Paired against `as_of`, every `post_x*` arm has 0 wins, 0 losses and all ties on both types and both
cut-offs (sign test p = 1). `no_as_of` against `as_of` on temporal-reasoning: 3 wins, 0 losses, 22
ties at @5 and @10 (p = 0.25).

Diagnostics:

- D1: knowledge-update 29 of 30 evidence sessions are valid at the question date, 0 questions
  without valid evidence. Temporal-reasoning 41 of 53 evidence sessions are valid; 3 of 25
  questions (`gpt4_5dcc0aab`, `gpt4_4edbafa2`, `c9f37c46`) have none, because every one of their
  5, 2 and 2 evidence sessions is dated after the question date.
- D2: the gap is exactly those 3 questions (3 of 3 with zero valid evidence). Among the 22
  temporal-reasoning questions with at least one valid evidence session, `as_of` and `no_as_of`
  hit the same 19.
- D3: 0 facts with `valid_to`, 0 supersession edges in the data.

Rule: recovery needed is ceil(3 / 2) = 2 questions at @5; every overfetch arm recovers 0. No-go.

Findings, kept to what was measured:

- The 0.88 against 0.76 gap is not post-filter dilution. Even the filter-after-top-k reference
  arm with no overfetch (`post_x1`) equals the shipped filter-before-top-k behavior, so on these 40
  questions the not-yet-valid facts never displaced valid evidence from the top 5 or 10. The
  gap is the benchmark labelling as evidence three questions' sessions that do not exist as of the
  question date. A retrieval change cannot recover them without ignoring `valid_from`, which is
  what `as_of` means. The 0.88 is therefore not a like-for-like target for the `as_of` row.
- H2 (closed validity windows) is untestable here: the data has no `valid_to` and no
  supersession, so it remains an untested guess.
- Expected outcome 1 was half wrong: the overfetch arms did equal `as_of` and never exceeded it, but
  `post_x1` was predicted to fall below `as_of` and did not. Expectations 2, 3 and 4 held.
- Scope: 40 questions, 3 of them decisive, one embedding-free lexical substrate, dev split only.
  This says nothing about the preprint's dense or ANN setting, or about data where future-dated
  facts are common or rank high; there a candidate overfetch could matter. The memory API already
  filters before top-k, so it is not exposed to the failure the preprint describes.

Decision: no `overfetch` option; no library code changed.

## Appendix: the 13 against 14 in the FPE write-up (knowledge-update `memory_recency_7d`, @5)

Per-query ranked ids and scores (all configs, 40 dev questions) were dumped with `--dump` from
builds of 12b49ed9 (before the posting cache, segment sizing and ingest counting changes) and
ae3fc4cb (main), three runs each. All six dumps are byte-identical (sha256 prefix
`9721bb5ebaf58295`), so those changes do not alter lexical results. Both give 13 at @5 and 14 at
@10 for that row. A build of 02f8b007, the commit that added the committed result, also gives 13 at
@5, so the committed 14 is not reproduced by the code at its own commit; the other 13 shared rows
match. The row is a boundary case: for `3ba21379` the evidence sessions rank 6th (score 0.3177) and
8th, just under the 5th hit (0.3203), so a small scoring difference at the time of the original run
flips it. Which exact build produced 14 was not determined.

Evidence for this appendix and for the pre-filter reading is committed as
`benchmarks/results/as_of_regression_check_dev.json` (one dump of 40 questions, the six dump
hashes, the per-commit rows, and the per-question `as_of` list lengths).

## Amendment: direct check of the filter-before-top-k reading

The protocol's reading of `recall.rs` depends on `constrained()`, which is true when any fact has
`valid_from` or `valid_to` set (`Stamp::windowed`); every fact here has `valid_from`. The reading
was also checked on the output: under filter-after-top-k at fetch k, an `as_of` list could only
contain ids from the unfiltered top 10. In 11 of 40 questions the `as_of` list contains ids outside
the unfiltered top 10, including `gpt4_5dcc0aab` (9 of its 10). Lists shorter than 10 (3 questions)
occur where fewer eligible facts match the lexical query; they are not a post-filter signature.
