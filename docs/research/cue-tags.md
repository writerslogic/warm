<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Unary change-report cue tags on STALE: kill test

Status: RUN COMPLETE, decision KILL. The protocol was committed (a428aa1b) before any statement
was labeled and before any new arm was run. Results are appended below it after the run.

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

# Results (appended after the run)

Run 2026-11-03. Labels: `benchmarks/cue_tags/labels.csv` (200 rows: sample id, stratum, label,
note; `weak` marks a borderline YES, `implicit` a NO where the change exists only relative to an
older statement, `planned` a NO for a stated plan). The labels were written before the tagger was
run on the sample. Measurements: `benchmarks/cue_tags/measure.py`, output
`benchmarks/results/cue_tags_measure_20261103.json`.

## Decision: KILL

Step 2 survives (precision 0.72, recall 0.76, selectivity 18.6% / 12.5%). Step 3 kills: on TEST
pooled CVA@1 the cue-gated extracted-slot arm e-x scores 19.2% against newest-wins b = 23.8%, a
lift of -4.6 points (95% paired cluster bootstrap -13.3 to +5.0), below the pre-registered +2.0
kill line and far from the +10 GO line. No library behaviour is changed. The oracle-slot arm e-o
reaches 52.9% (b + 29.2, interval +20.0 to +37.9), which clears b + 15 but, as pre-registered, is
an upper bound that needs the dataset's own conflict pair and is not a GO.

## Step 1 and 2: labels, precision, recall, selectivity

Base rate (YES on the random strata, n = 109): 11.0% (2.8% if `weak` labels count as NO).

| set | n | YES | flagged | precision | recall |
|---|---|---|---|---|---|
| all 200 | 200 | 58 | 61 | 0.72 | 0.76 |
| STALE (100) | 100 | 39 | 43 | 0.74 | 0.82 |
| LME-S dev (100) | 100 | 19 | 18 | 0.67 | 0.63 |
| all 200, `weak` as NO | 200 | 41 | 61 | 0.51 | 0.76 |
| random strata only | 109 | 12 | 13 | 0.54 | 0.58 |

Per stratum (primary labels): `stale_new` P 0.94 R 0.83 (n 40); `stale_old` 6 flags, 1 correct;
`stale_rand` 6 flags, 2 correct; `ku_new` P 0.75 R 0.50 (n 15); `ku_old` 7 flags, 4 correct;
`lme_rand` P 0.71 R 0.56 (n 69).

Fire rate on the newer conflict statement whatever its label: STALE M_new 77.5% (T1 80%, T2 75%);
LME-S knowledge-update newest evidence turn 26.7% (4 of 15).

Selectivity (flag rate over all user turns, unlabeled): STALE executed scope 18.6% of 35,584
turns (lex 9.8%, tense 10.3%); LME-S dev haystacks 12.5% of 24,390 turns.

Kill rule: precision 0.72 >= 0.50, recall 0.76 >= 0.60, selectivity 18.6% <= 40%. **The tagger
survives step 2.** Caveats, measured: precision falls to 0.51 when borderline YES labels are
counted as NO, and on the random strata alone it is 0.54 from 13 flags (too few to separate from
0.50); the 200-row precision is inflated by the positive-enriched `stale_new` stratum, as
expected. Recall on LME knowledge-update statements is low (4 of 15 newest evidence turns fire):
those updates are implicit counts ("I've tried four different ones so far", "20 dozen right now",
"my 5th project") that carry no change cue, so a unary lexical tag misses most of what
LongMemEval calls a knowledge update.

Errors (exploratory, the frozen list was not changed): misses are onset events without a listed
verb ("I just got the custody order back", "I finally signed the enrollment papers", "my new
Weber grill"); false positives are causal "since" ("since I find myself...") and habitual
"lately" on an unchanged state ("hovering around 150 pounds lately").

## Step 3: STALE arms

The example was rerun with `--dump` at the same flags (`--limit-dev 20 --limit-test 40`, probe
slice sha256 `4647ad00...`, embedder pack sha256 `b54d9369...`, both equal to the probe's). The
rerun's own result JSON equals the committed
`benchmarks/results/supersession_probe_stale_20261103.json` on every table, the tau sweep, the
bootstraps and the decision (real-vs-simulated mismatches 0). Simulation:
`benchmarks/cue_tags/stale_arms.py`, output `benchmarks/results/cue_tags_stale_arms_20261103.json`
(dump sha256 `c61addcb...`, 59.6 MB, kept outside git).

Reproduction gate (TEST pooled CVA@1, simulation vs the probe): a 3.75 / 3.75, b 23.75 / 23.75,
c 74.58 / 74.58, d at tau 0.75 9.58 / 9.58. Passed exactly.

Slot keys: STALE has none. e-x and e-x1 use EXTRACTED keys (embedding cosine to earlier facts);
e-o uses ORACLE keys (the dataset's M_old / M_new pair). The cue flags 72.5% of TEST M_new turns
(58 of 80) and 77.5% on DEV (31 of 40), and about 55 of the ~297 user turns per instance (18.6%).

DEV tau sweeps (pooled CVA@1; chosen tau in bold):

| tau | 0.60 | 0.65 | 0.70 | 0.75 | 0.80 | 0.85 | 0.90 | 0.95 | 1.01 |
|---|---|---|---|---|---|---|---|---|---|
| d (ungated) | 0.0 | 0.0 | 4.2 | **11.7** | 9.2 | 4.2 | 4.2 | 4.2 | 4.2 |
| e-x (gated, all >= tau) | 0.0 | 2.5 | 12.5 | **23.3** | 12.5 | 4.2 | 4.2 | 4.2 | 4.2 |
| e-x1 (gated, top-1) | 17.5 | **17.5** | 15.0 | 15.0 | 8.3 | 4.2 | 4.2 | 4.2 | 4.2 |

On DEV, e-x (23.3) was above b (20.0); on TEST it falls below b.

**TEST, pooled** (240 queries, 80 instances)

| arm | CVA@1 | Stale@1 | Stale@5 | new in top5 | CVA-session@1 | old-sibling@1 | CVA@1 T1 | CVA@1 T2 |
|---|---|---|---|---|---|---|---|---|
| (a) plain hybrid | 3.8 | 70.8 | 82.5 | 42.9 | 10.4 | 77.1 | 4.2 | 3.3 |
| (b) newest-wins among top 5 | 23.8 | 3.8 | 82.5 | 42.9 | 56.2 | 4.2 | 30.8 | 16.7 |
| (c) oracle links | 74.6 | 0.0 | 0.0 | 89.6 | 81.2 | 6.2 | 73.3 | 75.8 |
| (d) cosine links, tau 0.75 | 9.6 | 34.2 | 39.6 | 50.8 | 32.1 | 37.5 | 9.2 | 10.0 |
| (e-x) cue-gated cosine, tau 0.75 | 19.2 | 51.7 | 59.2 | 54.2 | 26.2 | 56.7 | 21.7 | 16.7 |
| (e-x1) cue-gated top-1, tau 0.65 | 12.9 | 54.6 | 63.8 | 55.4 | 24.2 | 59.6 | 14.2 | 11.7 |
| (e-o) cue-gated, oracle slot | 52.9 | 21.7 | 22.9 | 75.0 | 59.6 | 27.9 | 60.0 | 45.8 |

Paired cluster bootstrap on TEST (points of CVA@1, 95% interval, 10,000 resamples):

| x - y | points | interval |
|---|---|---|
| e-x - b | -4.6 | [-13.3, +5.0] |
| e-x - d | +9.6 | [+2.5, +17.1] |
| e-x - a | +15.4 | [+8.8, +22.9] |
| e-x1 - b | -10.8 | [-18.3, -2.9] |
| e-o - b | +29.2 | [+20.0, +37.9] |

Reading. Measured: gating cosine links by the cue doubles arm d (+9.6 points, interval excludes
0); it still loses to plain newest-wins over the top 5, and more than half of TEST queries still
put the old fact first (Stale@1 51.7% vs 3.8% for b). Inferred, not separately measured: the cue
removes many wrong links, but the flagged new turn usually does not reach the old one by cosine
(Type II shares no words with it), so the old fact keeps no successor. Measured: with the true pair as slot key the same flags give 52.9%,
close to the arithmetic a + 0.725 x (c - a) = 55.1. Inferred: the unary tag is not the
bottleneck; the slot (which earlier fact the change is about) is, and embedding similarity does
not recover it here. Conjecture: a slot extractor (subject/attribute keys from an LLM or a
trained model) would be needed, which is the non-LLM constraint this lever was meant to avoid.

## Step 4: LongMemEval-S dev wrong answers

14 wrong of 100 (`longmemeval_official_overnight_s_dev_gpt4o_k10.json`; 1 of the 14 is a
knowledge-update question; dev knowledge-update accuracy is 15 of 16). Read row by row against
the gold answer and the evidence turns of the cleaned source (no API calls):

- Wrong answer EQUAL to an older superseded value present in the reader's extraction: **0 of 14**.
- Wrong answer DERIVED from a superseded value the reader quoted: 2 of 14. `2133c1b5` (KU,
  "how long in Harajuku"): the reader quoted both "a month now" (April) and "3 months now"
  (October), distrusted the newer one, and answered "7 months" from the old one plus elapsed time.
  `7024f17c` ("jogging and yoga last week"): the reader used "I used to practice yoga three times
  a week, each time for 2 hours" despite "I've been slacking off" and answered 2 to 4 hours (gold
  0.5).
- Over-supersession, the opposite failure: 1 (`46a3abf7`, tanks: the reader decided the 5-gallon
  tank was replaced by the 20-gallon one and answered 2, gold 3).
- The other 11: counting (4), temporal arithmetic or event choice (4), preference rubric (2),
  missing assistant-side detail (1).

Unverified: presence in the retrieved top-10 is inferred from the reader's own quotes, because the
rows do not store the context and the score cache needed to rebuild it
(`~/.cache/warm-bench/longmemeval_s/runs/s-dev-small-all2`) is not on this machine. Supersession
bounds at most 2 to 3 of 14 dev errors (2 to 3 points of accuracy) on LongMemEval-S dev, and none
of them is a stale value repeated verbatim.

## Not tested

Labels from one labeler with no second rater (no agreement figure); 200 statements, 13 flags on
the random strata (population precision 0.54 is not separable from 0.50). Other cue lists (the
error analysis above is exploratory only); non-English text. The 163 paraphrase-only STALE
instances and the registered TEST instances beyond the executed 80. Slot extraction other than
embedding cosine (no subject/attribute extractor exists in the harness). LLM readers or judges,
STALE's own protocol. The LongMemEval retrieved context itself (proxy above). Any change to the
library.
