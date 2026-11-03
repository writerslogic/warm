# Matryoshka prefix retrieval: frozen development study

On this 100-query development sample, **256 dimensions with 100 candidates** and
**512 dimensions with 50 candidates** reproduced the full-vector ordered top ten
for every query. The 128-dimension/100-candidate route missed one full-vector hit.
These observations support investigating an opt-in implementation; they do not
establish a safe universal shortlist size or a recall guarantee.

This quality-only study tests short Qwen3 vectors for candidate search followed by
full-vector cosine reranking. It does not change WARM. The complete study is fixed
in `plan.json` before execution: 100 original queries, each question's own eligible
user-turn haystack, prefix dimensions 128/256/512/1024, candidate limits 20/50/100,
and ten final results. There is one frozen run, with no tuning after its results.

The source is previously used LongMemEval_S **development** data. All 100 queries,
including six abstention questions, count toward full-vector ranking agreement.
The 94 non-abstention questions count toward gold session metrics. Ten of these
have assistant-only evidence and no gold user turn in this user-only cache, so
turn metrics are undefined for those ten and use an explicitly reported denominator
of 84. Their IDs are retained in the plan and all 100 remain in agreement metrics.
Session metrics
count the distinct sessions represented in the ten returned turns; they are not a
separate top-ten-session search. The study measures retrieval, not answer accuracy.
Preparation verifies that every gold session for all 94 non-abstention questions
has at least one cached eligible user turn. Session-level evidence coverage does
not establish that a returned user turn itself contains the answer.

Both routes convert the same cached float16 vectors to float64 for arithmetic.
Every prefix is L2-normalized independently. Scores use a fixed reduction order;
ties sort by turn ID and then local row. The baseline scans all eligible full
1024-dimensional vectors. Each alternative scans its prefix, keeps its candidate
limit, and reranks that shortlist with the baseline's full cosine scores. Reusing
baseline scores is valid for this quality calculation and is not an implementation
or timing measurement. Reduced shortlists are approximate relative to full-vector
cosine even though prefix scoring is exact.

`mapping.json.gz` contains only question, session and turn identities, row ranges,
and existing gold-label identities. Preparation verifies their full order against
the cached source dataset and freezes the actual input hashes. Corpus text and
embedding arrays remain outside git. The historical exporter and pinned model
implementation are identified by commit and SHA-256 in `plan.json`; cached exports
do not carry a separate model-inference receipt. No source model is rerun here.

One source question repeats a session with two identical turn IDs. Preparation
requires identical source text and cached vector bytes before retaining the first
occurrence of each ID within that question. The original occurrence mapping and
the exact deduplication decision remain in the plan; no deduplication crosses
question boundaries.

The [pinned Qwen3 model card](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B/blob/97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3/README.md)
documents MRL dimensions from 32 through 1024. The
[Sentence Transformers Matryoshka documentation](https://sbert.net/examples/sentence_transformer/training/matryoshka/README.html)
describes truncating and normalizing these representations. Prefix dimensions
reduce candidate-scoring arithmetic; full vectors are still retained for
reranking, so the ratio is not a claim about total storage savings. Ordinary MRL
still runs the full embedding model. This cache used a 2048-token input cap and
cannot establish longer-context behavior, native Candle parity, held-out quality,
100K scaling, or an unconditional recall guarantee. It does not justify deleting
fact records.

Use the existing local competitive environment; these commands download nothing:

```sh
cd /Volumes/A/warm-matryoshka-eval
PYTHONDONTWRITEBYTECODE=1 uv run --offline --no-project /Volumes/A/.hms-bench-cache/competitive/envs/stdlib/bin/python docs/research/technical-audit-20261029/matryoshka-prefixes-20261102/test_probe.py
PYTHONDONTWRITEBYTECODE=1 uv run --offline --no-project /Volumes/A/.hms-bench-cache/competitive/envs/stdlib/bin/python docs/research/technical-audit-20261029/matryoshka-prefixes-20261102/prepare.py
PYTHONDONTWRITEBYTECODE=1 OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 uv run --offline --no-project /Volumes/A/.hms-bench-cache/competitive/envs/stdlib/bin/python docs/research/technical-audit-20261029/matryoshka-prefixes-20261102/probe.py
```

Preparation ran before signed/DCO plan commit
`d1b8853d599754be194d44986554e1128cc77153`; six independent reference/boundary
tests passed (`validation.log`). The single frozen run completed with no reruns or
parameter changes. Preparation and execution refuse to overwrite existing files.
To reproduce, use a fresh checkout at that plan commit with the same externally
cached inputs, run the tests and then `probe.py`; skip `prepare.py` because that
checkout already contains the frozen plan. Preserve the original results.

## Observed retrieval agreement

Each query has 214–288 eligible turns. The original 24,391 cached occurrences
become 24,389 eligible turn identities after the two verified within-question
duplicates are collapsed. Candidate coverage counts how many of the full-vector
top ten reach the shortlist. Here it also equals final top-ten set overlap after
full-vector reranking. Ordered agreement requires the same ten IDs in the same
order.

| Prefix dimensions | Candidates | Mean coverage | Worst query coverage | Identical ordered top ten | Missed full-top-ten hits |
|---:|---:|---:|---:|---:|---:|
| 128 | 20 | 90.7% | 40% | 55/100 | 93 |
| 128 | 50 | 99.2% | 70% | 94/100 | 8 |
| 128 | 100 | 99.9% | 90% | 99/100 | 1 |
| 256 | 20 | 94.8% | 60% | 67/100 | 52 |
| 256 | 50 | 99.8% | 90% | 98/100 | 2 |
| 256 | 100 | 100% | 100% | 100/100 | 0 |
| 512 | 20 | 98.7% | 80% | 89/100 | 13 |
| 512 | 50 | 100% | 100% | 100/100 | 0 |
| 512 | 100 | 100% | 100% | 100/100 | 0 |
| 1024 | 20 | 100% | 100% | 100/100 | 0 |
| 1024 | 50 | 100% | 100% | 100/100 | 0 |
| 1024 | 100 | 100% | 100% | 100/100 | 0 |

The 1024-dimensional rows are controls, since their candidate ranking is already
the full-vector ranking. The single 128/100 miss is question `352ab8bd`, turn
`noans_sharegpt_NoDZzot_7_4`. The 256/50 misses are question `gpt4_468eb064`, turn
`ultrachat_59629_5`, and question `352ab8bd`, turn `sharegpt_UCVg3jO_0_1`.
The worst 128/20 query is `gpt4_8279ba03`: six of its full-vector top ten fail to
reach the shortlist. `per-query.jsonl.gz` retains every route's final IDs, coverage,
missed IDs and gold metrics; `summary.json` lists every worst-query ID.

## Existing gold evidence coverage

These metrics use the same ten returned turns. “Any” means at least one gold
identity is represented; “all” means every gold identity is represented. The
six abstentions are excluded, and the ten questions without an eligible gold user
turn are additionally excluded only from turn metrics. Session metrics remain
coverage of an answer session, including sessions whose answer is assistant-only.

| Route | Any gold turn | All gold turns | Any gold session | All gold sessions |
|---|---:|---:|---:|---:|
| Full-vector baseline | 78/84 | 70/84 | 93/94 | 87/94 |
| 128 dimensions, 20 candidates | 79/84 | 70/84 | 94/94 | 88/94 |
| 256 dimensions, 20 candidates | 78/84 | 69/84 | 93/94 | 86/94 |
| Every other predeclared route | 78/84 | 70/84 | 93/94 | 87/94 |

Mean fractions of gold identities retrieved and deltas from the full-vector
baseline are retained in `summary.json`. Changed rankings can happen to improve
these labels while losing full-vector neighbors, as 128/20 does here. This small,
previously used development set does not establish superior quality or justify
choosing parameters after seeing the results.

The observed 256/100 and 512/50 routes merit further implementation evaluation,
with full vectors retained and approximate-search behavior explicit. Selecting
between them requires separate native and broader-corpus evidence. This study
measures neither execution speed nor storage footprint, and the small haystacks
do not support a scaling claim.

## Saved-result validation

`run.json` records the pre-run plan commit and exact output hashes. A separate
stdlib-only checker validated all 1,200 saved query/route records, input identity
membership, output hashes, full-dimension controls, gold denominators, missed IDs
and summary aggregates without recomputing vector scores. Its receipt is
`audit.json`. Recheck saved results with:

```sh
PYTHONDONTWRITEBYTECODE=1 uv run --offline --no-project /Volumes/A/.hms-bench-cache/competitive/envs/stdlib/bin/python docs/research/technical-audit-20261029/matryoshka-prefixes-20261102/check_results.py
```
