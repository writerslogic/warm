# R5 baseline: reproduction, evidence loss, and the selection/presentation experiment

Pre-registration: `PREREG.md` (committed before any reader call). Dev only; held-out ids are removed
by every script before reading rows or caches.

## Reproduction (measured)

- `prompts.py --system slice12i` regenerates the stored R5 dev rows: 228 of 228 `messages_sha256`
  (`check_rows.py equal`). Inputs: `warm_slice.py`, `warm_hybrid.py` (vendored verbatim from the
  private workspace), wheel `writerslogic_warm.abi3.so` sha256 `830fd0e7…` (2026-11-12),
  `embed_cache/` (44 MB, per-trajectory `.npy` + `.sig`; a cold re-embed was not tested), upstream
  LongMemEval-V2 commit `2cc8c540` (evaluation dir identical to the R5 stage copy).
- Reader environment: GCP disk snapshot `lme2-r5-snap-1115` of the R5 VM (vLLM 0.33.0,
  transformers 5.20.2, torch 2.14.0+cu130, Qwen3.5-9B revision `c2022362…`), `serve_r5.sh`.
  R5 ran `reader_stage_r5.py` for its first 32 dev questions and `reader_stage_rolling.py` for the
  rest; both send identical requests through upstream `call_reader_model_async`.
- Historical cache `reader_cache_slice12i.json`: 228 of 228 entries match the rows' hashes.

## Local scoring (measured, historical R5 outputs)

`score_local.py`: 149 locally scorable dev questions, 95 correct (63.8%); agrees with the recorded
per-question scores 149 of 149. 79 judge-required questions (65 abstention, 14 gotchas) are not
scored. Per-question: `r5_historical_local.json`.

## Commands

```sh
L=/Volumes/A/.hms-lb/lme2; PY=$L/venv/bin/python
$PY benchmarks/lme2/r5/prompts.py --system slice12i|slice12i_bound|slice12i_pagecover --domain web|enterprise --out <dir>
$PY benchmarks/lme2/r5/oracle_rows.py --condition B|D --out <dir>         # evaluator-only
uv run python benchmarks/lme2/r5/check_rows.py presented --a <R5 rows> --b <C rows>
uv run python benchmarks/lme2/r5/evidence_locate.py --out benchmarks/lme2/r5/evidence_locate.json
$PY benchmarks/lme2/r5/selection_eval.py --out <json>                    # memory-side, evaluator-only
uv run python benchmarks/lme2/r5/score_local.py --cache <reader cache> --out <json>
uv run python benchmarks/lme2/r5/paired.py --base A.json --new C.json [--ids ids.json]
uv run python benchmarks/lme2/r5/validate_runs.py --out <json>          # must exit 0 before scoring
uv run python benchmarks/lme2/r5/evidence_delivery.py --out benchmarks/lme2/r5/results/evidence_delivery_pilot.json
$PY benchmarks/lme2/r5/retrieval_trace.py --out benchmarks/lme2/r5/results/retrieval_trace_pilot.json
```

Reader runs: VM `lme2-reader-r5x` (us-central1-a, spot a2-ultragpu-1g from the snapshot),
`~/r5x/run_all.sh` and `~/r5x/run_cand.sh`, caches `~/r5x/cache_{A,B,C,D,P}.json`, copied to
`$L/r5x/out/`.

## Development boundary (checked 2026-11-20)

- Dispatched to the reader: `up/rows_{B,C,D,P}.jsonl` hold 35, 149, 35, 149 rows, none held out;
  `pilot_ids.json`, `rest_ids.json`, `local_ids.json` hold no held-out id. Condition A was
  dispatched from the R5 snapshot's own rows (`~/stage/rows/slice12i_*.jsonl`, all 451 questions,
  sha256 as the stored rows) restricted by `--only`; `validate_runs.py` checks that no cache holds a
  held-out id.
- Generated but not dispatched: before `prompts.py` was made development-only, the full-domain
  builds under `$L/r5x/{regen,slice12i_r5check,slice12i_bound,slice12i_pagecover,slice12i_pagecover_bound}_*`
  also built prompts for the 223 held-out questions (as the original R5 run did). Every script that
  reads them drops held-out ids on load; none was opened. Builds now take development ids only
  (`prompts.build` refuses held-out ids), and the frozen P payload is reproduced 149 of 149.
  `oracle_rows.py`, `selection_eval.py`, `evidence_locate.py`, `evidence_view.py`,
  `evidence_delivery.py`, `retrieval_trace.py` and `score_local.py` filter by `held_ids.json`.

## Independence of the 113-question decision split

The session record establishes that, before P was frozen, `selection_eval.py` was run over all 149
locally scored questions and its gold-phrase proxy totals were read, including the 113: R5 85 of
112 phrase questions, `pagecover` 89, `pagecover_t2` 90, `mmr` 88, `expand_r2` 79, and per-variant
gains and losses over all 112. P's configuration is the default written into `warm_select.py`
before any variant was scored, and it was not the best variant on that proxy; the choice cited
cohort-only numbers. No reader output on the 113 existed before the freeze. The 113 are
therefore a reader-evaluation holdback: no reader result on them shaped P, but their proxy labels
were seen during selection design, so they are not a design-independent validation.
