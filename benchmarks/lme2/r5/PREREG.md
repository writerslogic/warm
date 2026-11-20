# R5 selection and presentation experiment: pre-registration

Written and committed before any reader call of this experiment. Development questions only: every
script removes the ids in `../held_ids.json` before reading a row or cache entry.

## Baseline

Round 5 `slice12i` (dev 48.68 of 228 with the gpt-5.2 judge; see `../RUNS.md`). The stored prompt
rows (`round5/runs/slice12i_{web,enterprise}/prompt_rows.jsonl`, sha256 `84902687…` and
`2c6456e8…`) regenerate byte for byte (228 of 228 `messages_sha256`) from `warm_slice.py`
(`5e5618c4…`), `warm_hybrid.py` (`04f7fb12…`), the 2026-11-12 `writerslogic_warm` wheel
(`830fd0e7…`) and `embed_cache`. The historical reader cache matches those rows 228 of 228.

## Population and denominators

- Locally scored: the 149 dev questions whose eval function is `norm_phrase_set_match`,
  `norm_phrase_set_match_ordered`, `mc_choice_match` or `mc_choice_set_match`, scored by
  `score_local.py` with the pinned upstream evaluator. Historical R5: 95 of 149.
- Not scored: the 79 dev questions that need the hosted judge (`llm_abstention_checker` 65,
  `llm_gotchas_checker` 14). No heuristic replaces it; no combined score is reported.
- Pilot cohort (`pilot_cohort.json`, 36 ids, first ids per bucket by sha256 of
  `"r5-pilot-1|" + id` over the `evidence_locate.py` buckets): 27 R5 failures, 9 R5 successes.
- Rest: the other 113 locally scored questions. They were not used to design anything below.

## Conditions

| | selection | rendering | rows | n |
|---|---|---|---|---|
| A | R5 | R5 | stored R5 rows | 149 |
| B | annotated evidence states ranked first, then R5's ranking | R5 | `oracle_rows.py --condition B` | 35 |
| C | R5 (same states, same images, same kept AXTree text) | bound (`warm_bound.py`) | `prompts.py --system slice12i_bound` | 149 |
| D | as B | bound | `oracle_rows.py --condition D` | 35 |

- B and D are diagnostic oracles, evaluator-only, never headline results. Their evidence is the
  first evidence set of `annotations/batch_*.json`, checked by a second annotator who saw only
  those states (`annotations/verify_*.json`). `b7cc4da2` has no complete evidence
  (`sufficient_in_observations: unknown`) and has no B or D row.
- Budgets. Images: at most 6 screenshots per question in every condition (A and C carry the
  identical set; B and D take the first six ranked states, so evidence screenshots first). Text:
  A at most 320000 characters with the 60000-character state cap; C is A's kept text plus the
  binding labels (+1.4% characters on average, at most +2.3%, and no change to `memory_context`
  truncation); B and D at most the characters A delivered for the same question. Complete
  evidence that does not fit is recorded, not forced (882965b7 shows 1 of its 2 evidence states).

## Reader

Fixed reader, unchanged: Qwen/Qwen3.5-9B bf16 on vLLM 0.33.0 (transformers 5.20.2, torch
2.14.0+cu130, model revision `c202236235762e1c871ad0ccb60c8ee5ba337b9a`), restored from the R5 VM
disk snapshot `lme2-r5-snap-1115` on an A100-SXM4-80GB spot VM; `serve_r5.sh` settings; temperature
0.6, top-p 0.95, top-k 20, thinking on, 20000 completion tokens, no seed (as R5);
`reader_stage_r5.py`. One pass per question per condition. A row is retried only if the reader
produced no output (transport error); no row is rerun because of its answer.

A is run fresh. Fresh A is the control for B, C and D; historical A is never paired with them.
Fresh A against historical A measures the reader's run-to-run flip rate on identical inputs.

## Order

1. Pilot: A, B, C, D on the cohort. Check outputs are complete and parse; no comparison drives a
   change to B, C or D.
2. A and C on the 113 remaining questions.

## Decision rules (exact two-sided McNemar test on discordant pairs)

- Noise floor: discordant pairs of fresh A against historical A on 149.
- Selection is supported as a bottleneck if B beats fresh A on the 35 by a net of at least 5 with
  p ≤ 0.05.
- Presentation is supported if C beats fresh A on the 149 by a net of at least 5 with p ≤ 0.05;
  harmful if it loses by a net of at least 5 with p ≤ 0.05; otherwise not distinguished from noise.
- D against B and D against C are reported descriptively (35 questions).
- A production candidate is designed after the pilot without the annotations as input, frozen,
  and compared with fresh A on the 113 rest questions (headline split) and on all 149. It becomes a
  default only if it wins on the 113 by a net of at least 5 with p ≤ 0.05.

## Correction, made before any result of this experiment was read

The runs use `reader_stage_rolling.py` (the VM's `~/stage/warm_memory/reader_stage.py`, sha256
`d72aaaee…`), which R5 itself used for 196 of its 228 dev outputs, not `reader_stage_r5.py`. Both
send the same request through upstream `call_reader_model_async`; they differ only in scheduling
and in recording a bad request as an empty answer.

## Frozen candidate, recorded after the pilot and before any candidate reader call

Pilot (36 cohort questions; 35 for B and D): fresh A 14, B 25, C 12, D 27. B against fresh A:
+16/-4, p = 0.012, so selection is a supported bottleneck. C against fresh A: +2/-4, so the bound
rendering is not carried into the candidate.

Candidate P: `warm_select` with `select: pagecover` (`per_trajectory` 3, `identity_weight` 1.0),
R5 rendering, R5 budgets (`prompts.py --system slice12i_pagecover`). Chosen on the memory side
before the reader pilot ended; the annotations are not an input. One reader pass on the 113 rest
questions (headline split) and on the cohort, compared with fresh A.
