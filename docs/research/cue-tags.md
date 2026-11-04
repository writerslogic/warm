<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Unary change-report cue tags on STALE: kill test

Status: PROTOCOL. Written and committed before any statement was labeled and before any new arm
was run. Results are appended below it after the run.

## Question

The supersession probe (`docs/research/supersession-probe.md`) found that WARM's structure works
with oracle links (74.6% CVA@1 on TEST) and fails with cosine-detected links (9.6%), below
newest-wins among the top 5 (23.8%). The proposal under test (research notes r3 Lead 1, r2 L1;
no code was run there): the pairwise link is the hard part; a unary tag on the newer statement
("this reports a change to a prior state") is easier to recover with lexical cues, and only
flagged statements then need slot linking.

## Step 1: labels

200 statements, labeled by hand by reading (one labeler, the agent that ran this study; no model
call), drawn by `benchmarks/cue_tags/sample.py` (seed 20261103), text cut to 800 characters:

- STALE, DEV instances of the executed scope only (first 20 per type): `stale_new` = the M_new
  turn of all 40; `stale_old` = the M_old turn of 20; `stale_rand` = 40 other user turns.
- LongMemEval-S, dev split only (`benchmarks/public/longmemeval_split.json`, source sha256
  `d6f21ea9...`): `ku_new` / `ku_old` = latest-dated / earlier user turns with `has_answer` in
  the 16 dev knowledge-update questions (31 turns); `lme_rand` = 69 other dev haystack user turns.

Rubric. YES when the statement, read alone, reports that a state of the speaker (residence, job,
habit, preference, possession, schedule, health, relationship, plan, count or amount) is now
different from an earlier one, explicitly ("I switched to", "no longer") or by a stated onset
of a current state ("since the surgery my dose...", "I've started", "I just got a new"). NO when
it states a state with no change or onset, when the change is to someone or something unrelated
to the speaker's own ongoing state, or when the "change" is only a request ("help me switch").
Hypothetical or planned changes are NO. Labels are recorded before the tagger output is looked
at, in `benchmarks/cue_tags/labels.csv` (id, stratum, label, note).

Base rate: the YES rate on the random strata (`stale_rand`, `lme_rand`).

## Step 2: tagger and kill rule

`benchmarks/cue_tags/cue_tagger.py`, frozen at this commit: two regex groups (`lex`: change verbs
and phrases incl. a restricted "now"; `tense`: anchor-relative onsets). Flag = any pattern fires.
If the labels or the selectivity tempt a cue-list change, the decision still uses this frozen
list; any revised list is reported as exploratory only.

- Precision = P(label YES | flagged) over the 200 labeled statements, also per stratum.
- Recall = P(flagged | label YES) over the labeled statements, and separately on the newer
  conflict statements (`stale_new`, `ku_new`) whatever their label.
- Selectivity = flag rate over ALL user turns: every user turn of the 120 executed STALE
  instances, and every user turn of the 100 LME-S dev haystacks (unlabeled).

KILL if precision < 0.50, or recall < 0.60, or selectivity > 0.40 on either corpus.

## Step 3: STALE arms (only if step 2 survives)

Same data, embedder, store, ranking, scope (`--limit-dev 20 --limit-test 40`), metric (TEST pooled
CVA@1, 240 queries) and successor-following readout as the probe. The example is rerun with
`--dump` to export arm (a)'s top-50 lists and the cosine link candidates; every arm is then
simulated in `benchmarks/cue_tags/stale_arms.py`. This is exact because ranking with
`history = All` does not depend on links (the probe found 0 real-vs-simulated mismatches).

Reproduction gate: the simulation must give a = 3.8, b = 23.8, c = 74.6 and d (tau 0.75) = 9.6 on
TEST. If any differs, the new arms are not reported as results.

STALE ships no slot keys. Arms:

- (e-x) cue-gated cosine, EXTRACTED slot: a flagged fact x supersedes every strictly earlier
  fact y with cos(x, y) >= tau; unflagged facts link nothing. tau from the probe grid chosen on
  DEV by pooled CVA@1 (ties to the higher tau). This is arm (d) gated by the cue; e-x minus d is
  the cue's own contribution.
- (e-x1) secondary, not used for the decision: as e-x, but x supersedes only its single most
  similar earlier fact (cos >= tau, tau chosen on DEV the same way).
- (e-o) cue-gated, ORACLE slot: if the cue flags the M_new turn it supersedes M_old; nothing else
  links. This is near-arithmetic: each instance gets arm (c) when the cue fires on M_new and
  arm (a) otherwise, so e-o is about a + recall_on_M_new x (c - a). Precision plays no role. It
  is an upper bound, not a deployable system.

Decision (pre-registered, TEST pooled CVA@1, newest-wins b = 23.8):

- KILL if e-x - b < 2.0 points.
- GO if e-x - b >= 10 points (extracted slot keys). Only this can justify a library change.
- e-o is reported against b + 15 = 38.8 for completeness; a pass there alone is not a GO.
- Otherwise NO-GO.

## Step 4: LongMemEval wrong answers

For the 14 wrong answers of `benchmarks/results/longmemeval_official_overnight_s_dev_gpt4o_k10.json`:
how many wrong answers equal an older, superseded value of the asked attribute. The retrieved
top-10 context is not stored in the rows, and the score cache it was built from
(`~/.cache/warm-bench/longmemeval_s/runs/s-dev-small-all2`) is not on this machine, so presence in
the retrieved context is a proxy: the value appears in the reader's own extraction
(`hypothesis`), and an earlier-dated session of the full haystack states it while a later one
states the gold value. No API calls.
