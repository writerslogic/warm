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
```

Reader runs: VM `lme2-reader-r5x` (us-central1-a, spot a2-ultragpu-1g from the snapshot),
`~/r5x/run_all.sh`, caches `~/r5x/cache_{A,B,C,D}.json`, copied to `$L/r5x/`.
