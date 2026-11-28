# LoCoMo, Mem0-paper protocol, WARM 0.7.5 rerun

Run 2026-11-28 on an Apple M-series laptop. `warm-server` built from the v0.7.5 tag (58a816b6aedf787f25f2497ecc35972a6bea9ebd) with `--features server`; the harness is `benchmarks/public/locomo` at the same tag. Protocol as `benchmarks/results/locomo_v1.json`: Mem0's `ANSWER_PROMPT` and `ACCURACY_PROMPT` (mem0ai/mem0 `evaluation/` at 393a4fd5), `gpt-4o-mini-2024-07-18` as reader and judge at temperature 0, `text-embedding-3-small`, top_k=30 per speaker capped at 1,764 tokens, `locomo10.json` (snap-research/locomo 3eb6f2c), 10 conversations, categories 1 to 4 (1,540 questions), 5 reader and judge passes. One fact per raw turn, no LLM extraction, summaries or captions.

## Result

| | J (mean of 5 passes) | per-pass J | std |
|---|---|---|---|
| all 1,540 questions | 73.44 | 73.38, 73.31, 73.38, 73.70, 73.44 | 0.15 points |
| 7 held-out conversations (1,155) | 71.74 | 71.60, 71.95, 71.69, 71.86, 71.60 | 0.16 points |
| 3 tuning conversations (385) | 78.55 | 78.70, 77.40, 78.44, 79.22, 78.96 | 0.70 points |

By category, with the Mem0 paper's column labels (LoCoMo category ids 1, 3, 2, 4): Single-Hop 63.55, Multi-Hop 44.79, Temporal 71.78, Open-Domain 80.67. Multi-hop has 96 questions, so one question moves it by about one point. Mean context 1,749.4 tokens. Mean evidence recall 0.818. F1 51.92, BLEU-1 41.18.

The recall configuration was frozen on the three tuning conversations (conv-26, 30, 41), which are inside the headline; the held-out row is the clean estimate.

## Spend

2.8417 USD for the reader and judge passes (20,156,296 input tokens, 88,116 output tokens, 3,128,320 cached input tokens at list price) plus about 0.004 USD for the embeddings, under a 5 USD cap enforced by `--max-spend`.

## Identity check against the earlier build

`context-identity.txt`: the rendered reader prompts are byte-identical for 1,539 of 1,540 questions between commit c80aa52a and v0.7.5 (1,540 of 1,540 without embeddings). The one difference is conv-48 question 36, where two adjacent memories swap order.

## Files

| File | Content |
|---|---|
| `summary.json` | the harness summary: settings, per-pass scores, splits, per-category scores, token and spend totals |
| `predictions.jsonl` | one row per question: question, gold answer, retrieved ids, the 5 reader answers, the 5 judge verdicts, F1, BLEU-1 |
| `context-identity.txt` | the prompt comparison |
| `scripts/compare_contexts.py`, `scripts/dump-prompts.patch` | the comparison and the 4-line harness patch that stores the rendered prompt |

## Reproduce

```sh
git checkout v0.7.5
cargo build --release --features server --bin warm-server
uv run --no-project --with openai==1.109.1 --with nltk==3.9.1 --with tiktoken==0.12.0 --with jinja2==3.1.6 \
  python benchmarks/public/locomo/locomo_bench.py --server-bin target/release/warm-server \
  --data locomo10.json --convs all --embed --token-budget 1764 --runs 5 --max-spend 5 --out OUT
# add --dry-run first: it counts cache misses without a chat call (7,700 reader prompts, 17.1 million input tokens here)
```

Identity check: apply `scripts/dump-prompts.patch` to the harness, build `warm-server` at c80aa52a (the binary is `hms-server` there) and at v0.7.5, run the harness once per server with `--retrieval-only --embed --token-budget 1764 --out OUT_A` and `OUT_B` against the same `--cache` file so both see the same embeddings, then `python scripts/compare_contexts.py OUT_A/predictions.jsonl OUT_B/predictions.jsonl`.

The reader and judge call cache is not committed. Reader outputs at temperature 0 are not fully deterministic across runs, so a rerun can move the mean by a few tenths of a point.
