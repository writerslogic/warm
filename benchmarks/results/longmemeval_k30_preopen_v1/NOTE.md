# LongMemEval_S, WARM k30+preopen (benchmark-informed), three passes

Status: benchmark-informed. Reader, depth (k=30) and the "preopen" rule were chosen after the full 500-question results of the frozen run (`../longmemeval_500_official_v1.json`, 84.4%) were known. The 400 questions held out under that protocol are not held out for this configuration.

Result (official `evaluate_qa.py`, gpt-4o-mini judge, 500 questions; labels in `*.eval-results-gpt-4o-mini`):

| pass | `--repeat` | correct |
|---|---|---|
| r1 | 0 | 468 |
| r2 | 1 | 467 |
| r3 | 2 | 463 |

Mean 466.0/500 (93.2%). Majority-of-3 vote 470/500 (94.0%), computed from the three label files; it needs 3x the answering cost and is a separate configuration.

Recorded fields, unedited, in each report JSON:
- `reader`: `gpt-5-mini`; `reader_snapshots_observed`: `["gpt-5-mini"]`. No dated snapshot id was returned, so none is claimed. Default reasoning effort (`reasoning_effort` null), `api`: `sync`.
- `pinned_warm_revision`: `425d30e4`. This is the constant in the reader script's lineage (`longmemeval_official_qa.py` at 425d30e4), not the revision that produced the rankings. The rankings come from the frozen 500-run, whose report (`../longmemeval_500_official_v1.json`) pins `02468f57`.
- `rankings_sha256`: `edba139e7086ec44ebaa393bd948cf6f1c84de10c1b64a9e87964d2e576fbe66` (`lme500.rankings.json` here; label-blind top-50 turn ids per question, identical to the frozen run's).
- `official_commit`: `9e0b455f4ef0`; `evaluate_qa_sha256` begins `ecce9c4c79dc`; `split_sha256` begins `ef9c00e7`.
- `frozen_config_sha256` begins `fcf9621d`: the script's default `frozen/frozen_config.json`. These runs pass `--allow-nonfrozen`, so the frozen k=10/gpt-4o configuration (`../../public/longmemeval_500_frozen_config.json`, sha256 `7e81781011d9...`) does not apply to them.

Configuration: `--reader gpt-5-mini --topk 30 --pre 1 --pre-mode opening --rankings lme500.rankings.json --judges gpt-4o-mini`, one pass per `--repeat` value. `--pre-mode opening` prepends the preceding turn to a retrieved round only when that turn is the session's opening assistant turn. Launchers: `benchmarks/public/longmemeval_run_preopen.sh` (r1, r2) and `longmemeval_run_preopen_r3.sh` (r3). They contain the author's local paths and a local OpenAI-compatible proxy URL (`127.0.0.1:8787`, OpenRouter with the provider pinned to OpenAI); the proxy is not part of this repository. Reader script: `benchmarks/public/longmemeval_qa_l01.py` (copied byte for byte from the local `qa_l01.py`; it was run under that name from a working directory holding `frozen/` and `out/`).

Reproduction needs `longmemeval_s_cleaned.json` (xiaowu0162/longmemeval-cleaned) and OpenAI API access. The reader is not deterministic; a rerun will move a few questions.
