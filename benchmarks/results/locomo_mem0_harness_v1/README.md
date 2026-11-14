# LoCoMo on Mem0's own harness: WARM against Mem0 Platform's published per-question file

Recorded 2026-11-14. One WARM run, all 1,540 questions of categories 1 to 4. Prose and caveats: `docs/benchmarks/LOCOMO.md`.

## Result

`full1540-analysis.txt` (made by `scripts/analyze_full.py`): overall WARM 92.0 percent, Mem0 91.6 percent, paired difference
+0.5 pp [-1.0, +1.9] (bootstrap over questions, 10,000 resamples, seed 0, 95 percent percentile). Multi-hop -5.0 [-8.9, -1.1],
temporal +1.6 [-0.9, +4.4], open-domain -1.0 [-8.3, +5.2], single-hop +2.0 [+0.2, +3.8].

## Files

| File | Content |
|---|---|
| `per_question.jsonl` | one row per question: WARM answer, judge verdict, score and reason; Mem0's score; sha256 of the 200 retrieved fact ids; whether the gold evidence turns are among the 200 |
| `full1540-analysis.txt`, `misses_full.json` | the comparison and the 58 + 65 disagreements with a gold-evidence class each |
| `ingest-gate.txt`, `scripts/gate.py` | per-conversation ingest check: dataset turns = chunks processed = server remember calls (200) = 0 failures, plus a spot recall |
| `run.py.diff` | the only change to Mem0's `benchmarks/locomo/run.py` (questions of a conversation run concurrently; prompts, retrieval and judging untouched) |
| `scripts/run_pilot.py`, `scripts/run_full.py` | the driver (spend guard, prompt-hash call cache, usage log) and the orchestration that started the server, ingested, gated and answered |
| `all-usage.jsonl`, `all-child-env-names.txt` | one line per paid call (tokens, USD, prompt hash; no response bodies or ids) and the environment variable names the harness process saw |
| `empty_answers.txt`, `scripts/empty_answers.py` | the 22 questions whose answer came back empty (see the LOCOMO.md section) |
| `mem0_published/` | Mem0's per-question file, unmodified, with its licence (below) |
| `retrieval_multihop/` | free offline retrieval study of the multi-hop misses: pre-registration, scripts, results |

The paid-call response cache (`call-cache.jsonl`) is not committed: it holds raw API envelopes with response ids.

## Provenance

- Mem0 harness: `mem0ai/memory-benchmarks` at `4b61c5d31b9c668a12b4f5e78064248a02c82d2b`, `benchmarks/locomo/run.py` plus `run.py.diff`.
- Mem0's published numbers: `results/platform/locomo_results.json` in that repository, sha256
  `36338fa6c1ca38bcf9e3fc33a5cbc3b6e53bdc4bafaeeaee0947cf13b5527911`. A verbatim copy is in `mem0_published/`. The repository
  is Apache-2.0 (`mem0_published/LICENSE-memory-benchmarks-Apache-2.0`); the file is copyright its authors, redistributed
  unchanged under that licence. Its metadata: provider azure, answerer and judge `gpt-5`, project `locomo-v3-run`, timestamp
  20260406_170942, cutoff top_200 only.
- Dataset: `locomo10.json` from snap-research/locomo (see `docs/benchmarks/LOCOMO.md`, Setup).
- Models: answerer and judge requested as `gpt-5`, returned `gpt-5-2025-08-07` on every call, through the OpenAI API.
  Embeddings `text-embedding-3-small`, 1,536 dimensions, for turns and queries.
- WARM: `warm-server` 0.6.8, defaults, no tuning; the `src/` tree is identical to this repository at 29fd6dc (checked with
  `diff -r`); binary built as `pilot-memsrv` from that tree.
- Spend: 41.5538 USD over 2,962 paid calls (`all-usage.jsonl`; 2,962 includes retries and truncated answers).

## Commands

Ingest, gate and answer (run from `scripts/`; the key file path is read by `run_pilot.py` and never printed):

```sh
python scripts/run_full.py
# phase 1: run_pilot.py all 45 locomo10.json OUT --max-questions 0     (ingest only)
# gate:    gate.py OUT                                                  (-> ingest-gate.txt)
# phase 2: run_pilot.py all 45 locomo10.json OUT                        (answer and judge)
# run_pilot.py calls Mem0's harness as: --top-k 200 --top-k-cutoffs 200 --max-workers 16
#   --answerer-model gpt-5 --judge-model gpt-5 --provider openai --conversations 0,...,9
# warm-server flags: --no-auth --input-dim 1536 --max-top-k 200 --max-queue 1000
```

Analysis (free): `uv run --no-project --with numpy python scripts/analyze_full.py` after pointing its paths at the run output;
`scripts/empty_answers.py` runs inside this directory.
