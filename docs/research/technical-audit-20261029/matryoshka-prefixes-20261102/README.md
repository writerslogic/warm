# Matryoshka prefix retrieval: frozen development study

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

The first two commands validate fixtures and freeze a fresh study. Commit the plan
before the third command. Preparation and execution refuse to overwrite existing
plan/results. Reproduction uses a fresh checkout at the plan commit and the same
externally cached inputs; preserve the original result directory.

Results are pending the signed plan commit and single frozen run.
