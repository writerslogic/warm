<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Supersession probe on STALE: kill test

Status: RUN COMPLETE, decision NO-GO. The protocol was committed before any arm was run. Results are appended
below it, after the run, without editing the protocol.

## Question

Agent-memory leaderboards are crowded; supersession (the newest valid fact replaces an older one,
stale facts must not come back) may not be. Does WARM's exact structure (`supersedes`,
`valid_from`) give a large retrieval-level margin over plain hybrid retrieval on STALE, without an
LLM reader?

## Data (public, used as is)

- STALE, arXiv 2605.06527 (abstract and HTML appendices read; the PDF body was not).
- Hugging Face dataset `STALEproj/STALE`, revision `617c51dc200b5ab09970834144c7e51c77959af0`,
  file `T1_T2_400_FULL.json` (305,908,212 bytes), sha256
  `5f3ec375179e20e2e94469e018189188f34e2e7e5f21cbecbd99fcfa648c1876` (equals the LFS oid;
  verified after download). Code: `github.com/icedreamc/STALE`.
- License: CC BY 4.0. Distractor sessions derive from LongMemEval (MIT).
- 400 instances (200 Type I, 200 Type II), 50 sessions each, one conflict pair (`M_old` in
  `relevant_session_index[0]`, `M_new` in `[1]`, always later), three probe queries per instance
  (`dim1` state resolution, `dim2` premise resistance, `dim3` implicit policy adaptation).

## What this is NOT

STALE grades free-form answers with an LLM judge (Gemini-3.1-flash-lite) on whether the answer
shows awareness of the conflict and the updated state. The published numbers are end-to-end:
55.2% best model (Gemini-3.1-pro) and 5 to 18% for memory frameworks on GPT-4o-mini. The dataset
has no gold current-value answers. This probe is a retrieval-level proxy on the public data:
**it is not comparable to those numbers**, and a high score here does not mean WARM would score
high on STALE's own protocol. No LLM reader is used.

## Facts and ground truth

- A fact is one user turn (`role == user`) of any of the 50 sessions of an instance. Assistant
  turns are not stored. `valid_from = event_time =` the session timestamp (UTC, ms).
- Eligible instance: `M_old` is an exact substring of a user turn of the old session and
  `M_new` an exact substring of a user turn of the new session. Of 400 instances, 237 qualify
  (118 Type I, 119 Type II). The other 163 paraphrase the observation and are excluded; this is a
  selection effect and is reported as such.
- `old_id` is that old turn, `new_id` that new turn. A query is answered correctly when the top-1
  returned fact is `new_id`.
- Each instance gets its own store (the conflict is per user history). Queries are the three
  probe queries verbatim.

## Split

By instance, stratified by type, seed `20261103`, order `sha256("20261103|" + uid)`. Per type the
first 40 eligible instances are DEV, the next 78 are TEST (Type I: 118 = 40 + 78; Type II:
119 = 40 + 79, one unused). Dev: 80 instances (240 queries). Test: 156 instances (468 queries).
Everything chosen from data (the threshold of arm d) is chosen on DEV only. Slicing script:
`benchmarks/supersession_probe/prep.py`.

## Retrieval stack (identical for arms a, c, d)

- Embedder: the repo's `local-embed` engine, `bge-small-en-v1.5` (revision
  `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a`, MIT), int8 pack converted offline from the local
  Hugging Face snapshot with the registry settings (CLS pooling, query prefix
  `Represent this sentence for searching relevant passages: `, max length 512). Facts embedded as
  passages, queries as queries. Pack sha256 is recorded in the result JSON.
- Memory API `Memory::new`, `WarmCore` dimension 4096 with the pack's embedding space, document
  search defaults (`fusion = blend`, lexical weight 1, semantic weight 1), `candidate_limit` 100,
  `as_of` fixed at 2026-01-01 UTC.
- Ranked list for a query: `recall(history = All, k = 50)`. With `history = All` ranking is
  unchanged by links, which is what lets arm d be tuned by simulation on DEV (see below).

## Arms

- (a) Plain hybrid: facts stored with no links; top-k of the ranked list.
- (b) Recency: the top 5 of (a) re-sorted by session timestamp, newest first (ties keep rank).
- (c) WARM, ORACLE links (upper bound, uses dataset labels, not a deployable system): the
  `new_id` fact is remembered with `supersedes = [old_id]`. Primary readout, "successor
  following" (harness logic over WARM's returned links, not engine behavior): walk the ranked
  list; a current hit is kept; a superseded hit is replaced by the newest current fact reachable
  through `superseded_by` (newest `effective_from`, ties by id); duplicates dropped; first 5.
  Secondary readout (c-filter): stock `recall(history = Current, k = 5)`, which only filters.
- (d) WARM, DETECTED links, no LLM: sessions ingested chronologically; when a fact x arrives,
  every already-stored fact y with `ts_y < ts_x` (strictly earlier session) and
  `cos(x, y) >= tau` is listed in x's `supersedes`. Same successor-following readout as (c).
  `tau` is chosen on DEV from {0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.01 (no links)}
  by pooled CVA@1 (ties go to the higher tau). DEV values come from simulating the links over
  arm (a)'s ranked lists, which is exact because ranking does not depend on links. The TEST run of
  (d) uses real WARM stores with the chosen tau; the simulation is also computed on TEST and any
  disagreement with the real run is reported.

## Metrics

Per query, over the returned list L (at most 5):

- **CVA@1** (primary): `L[0] == new_id`.
- Stale@1 and Stale@5: `old_id` in `L[:1]`, `L[:5]`.
- CVA-session@1: `L[0]` is any turn of the new session. Old-sibling@1: `L[0]` is any turn of the
  old session (explains low scores).
- `new_id` in `L[:5]`.

Reported pooled, per type (T1, T2) and per dim, with a paired cluster bootstrap over instances
(10,000 resamples, seed `20261103`) for differences. Pooled = all test queries equally weighted.

## Decision rule (fixed now)

Computed on TEST, pooled CVA@1, in this order:

1. INCONCLUSIVE if arm (a) >= 90%: the probe is too easy for the baseline.
2. GO only if (d) - (a) >= 15 points absolute AND arm (c) >= 80% CVA@1. The second condition is
   this study's operationalization of "the logic is not the bottleneck"; it is not from any paper.
3. Otherwise NO-GO.

Not part of the rule but reported: if (b) is within 5 points of (d), the headroom is recency,
not WARM structure. The newest session is later than the old one in all 400 instances, so
recency has no adversarial case here.

## Not tested

- Real conversational extraction of facts: facts are whole user turns and the conflict turn is
  located by exact substring, not extracted by any model.
- The paper's own models, its judge, and its scoring protocol; STALE's end-to-end accuracy.
- LLM readers of any kind, and CUPMem.
- The 163 paraphrase-only instances.
- Assistant turns as evidence, context lengths beyond the 50 sessions shipped, other embedders,
  other fusion weights, other lexical/semantic splits.
- Premise Resistance and Implicit Policy Adaptation as behaviors: only the retrieval of the
  current evidence for those queries is measured.

## Amendment A (committed before the full run, after a 2-instance smoke test)

Embedding 53,000 unique turns at about 32 per second on a shared, loaded machine would not fit
the time budget. The executed scope is therefore a prefix of the registered split, in the same
hash order: DEV = first 20 per type (40 instances, 120 queries), TEST = first 40 per type
(80 instances, 240 queries). Run flags: `--limit-dev 20 --limit-test 40`. Arm (c) is computed on
TEST only (DEV does not need it; the tau choice for arm d uses arms a and the simulation).
Nothing else in the protocol changes, including the decision rule. The smoke test (2 instances
per split and type) was a wiring check; its numbers are not used. Tie-break for "newest current
successor": newest `effective_from`, then smallest id.
Consequence: 240 test queries from 80 instances gives wide intervals (about plus or minus 6 points
on a pooled rate, wider per type); a difference near the 15-point line is not settled by this run.

# Results (appended after the run)

Run: 2026-11-03, `examples/supersession_probe.rs` at commit `0925696b`, flags
`--limit-dev 20 --limit-test 40`, result file
`benchmarks/results/supersession_probe_stale_20261103.json` (seed 20261103; STALE file sha256,
probe slice sha256 and embedder pack sha256 are recorded inside it). The pack is converted
locally from the pinned `bge-small-en-v1.5` snapshot (the three input files match the registry
sha256 values; the pack hash differs from the registry's, which was not investigated). Scope:
DEV 40 instances, TEST 80 instances (40 per type), 240 test queries. Retrieval-level proxy on
237 of 400 STALE instances: **not comparable to STALE's LLM-judged 55.2% (Gemini-3.1-pro) or the
5 to 18% of the memory frameworks (GPT-4o-mini)**. Tables are produced by
`benchmarks/supersession_probe/tables.py`.

## Decision: NO-GO

By the pre-registered rule on TEST pooled CVA@1: (a) 3.8%, (d) 9.6%, so d - a = +5.8 points
(95% cluster-bootstrap interval +1.2 to +11.2), well short of the required +15. Arm (c) reaches
74.6%, below the 80% operationalization, so the second condition also fails. Arm (a) is far below
90%, so the result is not INCONCLUSIVE. The interval excludes +15 on the pooled data.

**TEST, Pooled** (n queries = 240)

| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 |
|---|---|---|---|---|---|---|
| (a) plain hybrid | 3.8 | 70.8 | 82.5 | 42.9 | 10.4 | 77.1 |
| (b) recency among top5 | 23.8 | 3.8 | 82.5 | 42.9 | 56.2 | 4.2 |
| (c) WARM oracle links, follow | 74.6 | 0.0 | 0.0 | 89.6 | 81.2 | 6.2 |
| (c-filter) oracle, stock filter | 7.5 | 0.0 | 0.0 | 47.9 | 20.8 | 55.8 |
| (d) WARM detected links, follow | 9.6 | 34.2 | 39.6 | 50.8 | 32.1 | 37.5 |

**TEST, Type I** (n queries = 120)

| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 |
|---|---|---|---|---|---|---|
| (a) plain hybrid | 4.2 | 69.2 | 82.5 | 55.8 | 15.0 | 76.7 |
| (b) recency among top5 | 30.8 | 2.5 | 82.5 | 55.8 | 62.5 | 3.3 |
| (c) WARM oracle links, follow | 73.3 | 0.0 | 0.0 | 91.7 | 84.2 | 7.5 |
| (c-filter) oracle, stock filter | 11.7 | 0.0 | 0.0 | 59.2 | 30.8 | 53.3 |
| (d) WARM detected links, follow | 9.2 | 26.7 | 31.7 | 57.5 | 36.7 | 30.8 |

**TEST, Type II** (n queries = 120)

| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 |
|---|---|---|---|---|---|---|
| (a) plain hybrid | 3.3 | 72.5 | 82.5 | 30.0 | 5.8 | 77.5 |
| (b) recency among top5 | 16.7 | 5.0 | 82.5 | 30.0 | 50.0 | 5.0 |
| (c) WARM oracle links, follow | 75.8 | 0.0 | 0.0 | 87.5 | 78.3 | 5.0 |
| (c-filter) oracle, stock filter | 3.3 | 0.0 | 0.0 | 36.7 | 10.8 | 58.3 |
| (d) WARM detected links, follow | 10.0 | 41.7 | 47.5 | 44.2 | 27.5 | 44.2 |

**TEST CVA@1 by dim (pooled)**

| arm | dim1 | dim2 | dim3 |
|---|---|---|---|
| (a) plain hybrid | 0.0 | 0.0 | 11.2 |
| (b) recency among top5 | 21.2 | 32.5 | 17.5 |
| (c) WARM oracle links, follow | 97.5 | 98.8 | 27.5 |
| (c-filter) oracle, stock filter | 3.8 | 7.5 | 11.2 |
| (d) WARM detected links, follow | 7.5 | 7.5 | 13.8 |

**DEV (arms a, b only; d is the simulation sweep below)**

| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 |
|---|---|---|---|---|---|---|
| (a) plain hybrid | 4.2 | 70.0 | 84.2 | 36.7 | 8.3 | 80.0 |
| (b) recency among top5 | 20.0 | 8.3 | 84.2 | 36.7 | 50.0 | 9.2 |

**tau sweep on DEV (simulated arm d, pooled CVA@1)**

| tau | CVA@1 |
|---|---|
| 0.60 | 0.0 |
| 0.65 | 0.0 |
| 0.70 | 4.2 |
| 0.75 | 11.7 |
| 0.80 | 9.2 |
| 0.85 | 4.2 |
| 0.90 | 4.2 |
| 0.95 | 4.2 |
| 1.01 | 4.2 |

Chosen tau = 0.75.

**Paired cluster bootstrap on TEST (points of CVA@1, 95% interval over instances)**

| x - y | pooled | T1 | T2 |
|---|---|---|---|
| d - a | +5.8 [+1.2, +11.2] | +5.0 [-0.8, +12.5] | +6.7 [+0.0, +15.0] |
| c - a | +70.8 [+67.5, +74.2] | +69.2 [+64.2, +74.2] | +72.5 [+69.2, +76.7] |
| b - a | +20.0 [+14.2, +26.2] | +26.7 [+16.7, +36.7] | +13.3 [+7.5, +19.2] |
| d - b | -14.2 [-21.7, -6.7] | -21.7 [-31.7, -11.7] | -6.7 [-16.7, +4.2] |

Decision: **NO-GO** (a=3.8, b=23.8, c=74.6, d=9.6). Real-vs-simulated arm d list mismatches on TEST: 0.

## Reading (measurement, inference, conjecture kept apart)

Measured:

- Plain hybrid retrieval almost never puts the new fact first: for 77% of queries the top-1 is a
  turn of the old session (Stale@1 70.8%). The probe is hard for arm (a), not easy.
- With oracle links and successor following, queries that name the stale state (dim1, dim2) reach
  97.5% and 98.8% CVA@1. Dim3 queries ("recommend a commute plan") name neither state, the old
  fact is usually not a hit, so links cannot redirect and the oracle gets 27.5%. That dim alone
  puts arm (c) under 80%.
- The stock `recall` (history = Current) only filters superseded facts; with the same oracle
  links it reaches 7.5%. The 74.6% needs the successor-following step, which lives in this
  harness, not in the engine.
- The write-time rule (embedding cosine at the DEV-chosen 0.75, later session supersedes) gives
  9.6%. No tau in the grid is useful on DEV (best 11.7% simulated; DEV arm a is 4.2%). The
  simulation and the real stores agreed on every TEST list (0 mismatches).
- A recency heuristic over the top 5 (arm b) reaches 23.8%, 14 points above arm (d).
  Because the new session is always later here, this favors recency; it is not evidence that
  recency solves supersession in general.

Inferred (not separately tested): the old and new fact share little surface or embedding
similarity in this data (a different city, or an upstream change with no shared words in Type II),
while same-topic turns elsewhere in the haystack score as high, so a threshold rule cannot
separate the conflict pair from topical neighbors. Both types score about the same for (d)
(9.2% and 10.0%).

Conjecture: headroom exists only if supersession detection is solved (the oracle shows the
storage and retrieval structure is not the limit for state-naming queries). A detector that
reasons about states (an NLI or LLM-based rule) might move arm (d) toward arm (c). That is the
same unsolved problem STALE reports, so "exact logic beats model-only memory without an LLM"
is not supported by this probe.

## Not tested

All items in the pre-registered list above, plus: the 163 paraphrase-only instances; the 120
registered instances beyond the executed prefix (TEST was 80 of the registered 156); arm (c) on
DEV; any detector other than embedding cosine; other embedders, fusion weights, or `k`; an
LLM reader; assistant turns as evidence. A cosine floor of 0.60 to 0.95 was swept on DEV only; no
test-time threshold sweep was run, so a threshold chosen with test knowledge is untested. TEST
CIs are wide (80 instances); the pooled interval for d - a excludes +15, the per-type ones are
wider (Type II +0.0 to +15.0).
