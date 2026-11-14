# LoCoMo head-to-head: retrieval round 3, Mem0's own harness, adapters, cost

Status 2026-11-14. This page records free work only. No paid model call was made for any number below, and no score in
[LOCOMO.md](LOCOMO.md) changed. Per-question rows, scripts and logs are in the campaign evidence directory
`locomo-20261114/`.

Update: the paid run this page costed was made afterwards and is recorded in [LOCOMO.md](LOCOMO.md) ("Mem0's own harness, top 200, gpt-5"),
with its evidence in `benchmarks/results/locomo_mem0_harness_v1/`. Its cost was 41.55 USD, not the 57.57 estimated in section 4.

## 1. Two protocols, two numbers

[LOCOMO.md](LOCOMO.md) compares WARM with the Mem0 paper's 66.88 under the paper-era protocol. Mem0 now publishes its own harness
(`mem0ai/memory-benchmarks` at 4b61c5d, 2026-05-13) and a Platform "v3" LoCoMo score that its README gives as 92.5 percent (1,425 of 1,540) at top-200. The per-question file it
publishes (`results/platform/locomo_results.json`, top-200 only) sums to 1,410 correct, 91.56 percent, with 1 error, and the
README's per-category figures are averages over cutoffs 10 to 200 that the file cannot reproduce. The two protocols are not
comparable. Read from the harness source and that file:

| | LOCOMO.md protocol (paper era) | Mem0 `memory-benchmarks` |
|---|---|---|
| Questions | 1,540, categories 1-4 | same 1,540 |
| Context | 1,764 o200k tokens (WARM) / Mem0's reported 1,764 | top-200 extracted memories, chronological; Mem0's own token count is not published (the file holds no memory text). WARM with raw turns is about 8,000 |
| Memory lists | one list per speaker | one list per conversation (`user_id`) |
| Reader | gpt-4o-mini, short-answer prompt | gpt-5 (published run, Azure) or default gpt-4o, a seven-step reasoning prompt that also receives the conversation date range |
| Judge | `ACCURACY_PROMPT` | unified rubric: any one correct item in a list is CORRECT, dates within 14 days and durations within 50 percent are CORRECT, paraphrase and same-referent leniency; score >= 0.5 |
| Evidence to judge | no | optional `--with-evidence`, off in the published run |
| Published run | 5 passes | one pass per cutoff; metadata lists 156 questions merged in from re-runs |

The published Platform run used only the top-200 cutoff. The harness default evaluates cutoffs 10, 20, 50 and 200, which
multiplies reader and judge calls by four. Category 3 (open-domain) is the weakest category there too (76.0 percent at top-200).

## 2. Retrieval round 3 (development conversations only)

Pre-registration: `PREREG.md` in the evidence directory, written before any variant ran. Development set: conv-26, conv-30,
conv-41 (74 multi-hop, 19 open-domain questions with evidence). The seven held-out conversations were not read. Embeddings came
from the existing cache with a hard failure on a miss. Tokens are counted as the harness counts them: o200k over
`json.dumps(item)` plus 1 per memory for the cap, and o200k over `json.dumps(list, indent=4)` for the reported context.
Mem0 never documents a tokenizer for its 1,764. The baseline run reproduced the stored retrieval of all 385 dev questions exactly.

Eight configurations were tried. Intervals are paired bootstrap, 10,000 resamples, 95 percent, over questions.

| Configuration | multi-hop recall (change) | multi-hop all-gold (change) | worst other category | result |
|---|---|---|---|---|
| base (round-robin, cap 1,764) | .642 | .392 | | |
| skip memories that do not fit | +.000 | +.000 | 0 | no effect |
| render without the `Speaker:` prefix | +.010 [.000, .027] | +.014 [.000, .041] | single-hop -.005 | rejected |
| prefix off and skip | +.010 [.000, .027] | +.014 [.000, .041] | single-hop -.005 | rejected |
| prefix off, skip, k=60 | +.010 [.000, .027] | +.014 [.000, .041] | single-hop -.005 | rejected |
| second hop, alpha .5 | +.003 [-.014, .022] | +.000 | single-hop -.005 | rejected |
| second hop, alpha 1.0 | +.000 [-.023, .024] | +.000 | temporal -.011, single-hop -.015 | rejected |
| prefix off, skip, second hop .5 | +.006 [-.007, .023] | +.000 | none | rejected (interval includes zero) |

Keep rule: multi-hop recall interval excludes zero, and no category's point estimate falls. Nothing passed. No engine or
harness retrieval change is kept. Open-domain moved only under second hop (+.026 [.000, .079], 19 questions), which is within what
19 questions can show. Median retrieval latency: base 15 ms, second hop 36 to 42 ms.

Exploratory, not pre-registered, so not eligible to be kept: one merged relevance list for both speakers under the same cap
gives multi-hop recall +.038 [+.007, +.074] and all-gold +.054 [.000, +.122], with single-hop -.005 [-.025, +.015]. This is the
cosine-packing family that round 2 rejected on the LongMemEval gate. It needs a held-out look and that gate before it can ship.

Evidence recall under Mem0's retrieval shape (one tenant per conversation, units are raw turns, recall `k` raised with
`--max-top-k 200`; the server default of 100 rejects k=200). Development set, all-gold in brackets:

| units | tokens (approx.) | multi-hop | open-domain | single-hop | temporal |
|---|---|---|---|---|---|
| top-10 | 420 | .495 (.243) | .316 (.211) | .780 (.775) | .911 (.911) |
| top-20 | 840 | .585 (.351) | .421 (.368) | .835 (.830) | .933 (.933) |
| top-50 | 2,080 | .754 (.541) | .605 (.526) | .920 (.915) | .956 (.956) |
| top-200 | 8,150 | .938 (.824) | .864 (.737) | .968 (.965) | .989 (.989) |

The probe ingests caption-free turns (the paper-era protocol). Mem0's harness appends image tags to turns (`session_to_chunks`),
which `WarmClient` therefore also ingests, so units there are longer and retrieval differs; round 2 measured that effect. At
top-200 the multi-hop all-gold rate is 0.82 [.73, .91], so 18 percent of development multi-hop questions still lack some
evidence. Whether the reader or the lenient judge matters more under this harness is untested.

## 3. Adapters

`locomo_bench.py --system {warm,mem0ai,graphiti}` runs the same reader, judge, prompt, cap and token accounting for all three.
`--llm-base-url` and `--llm-api-key-env` set the endpoint and key variable; `stub_llm.py` is an OpenAI-compatible stub that
counts calls and prompt tokens and costs nothing. The mem0ai adapter removes `OPENROUTER_API_KEY` from the environment
because mem0ai sends every chat call to OpenRouter whenever that variable is set, whatever `openai_base_url` says. Always run
adapters with `env -i`. The cost model's 37.7 tokens per memory unit (section 4) also assumes caption-free turns.

| System | Version | Settings | Plumbing status |
|---|---|---|---|
| mem0ai OSS | 2.2.1 (`mem0ai[nlp,extras]`) | paper-era ingest (one user per speaker, batches of 2), Qdrant local, `text-embedding-3-small`, `top_k=30`, default threshold | harness run with the stub covers ingest, reader and judge, but search returned nothing there (default threshold 0.1 against random stub vectors); search was shown separately with `threshold=0` |
| Graphiti | graphiti-core 0.31.2, Kuzu 0.11.3 | one message episode per turn, Zep template for facts and entities, edge search 20, node search 20 | episode 1 reaches both extraction calls; the embedded Kuzu driver then fails (`edge_name_and_fact` full-text index missing), and `group_id` with Kuzu raises `AttributeError`. Needs Neo4j (not installed here) for a real run |
| WARM behind Mem0's own harness | `mem0_harness_warm.py` + `run_mem0_harness.py` | `WarmClient` replaces `Mem0Client`; raw turn per message; top-k recall | Mem0's unmodified `run.py` ran end to end with the stub (6 questions, cutoffs 10 and 50) |

Measured with the stub, mem0ai ingest costs one extraction call per `add` of about 8,100 prompt tokens (36 calls, 291,805
tokens, 2 sessions). Graphiti's first episode needs 2 calls of 1,890 and 1,500 prompt tokens. Real model output changes
how many entities and edges follow, so call counts are lower bounds and Graphiti beyond episode 1 is modelled, not measured.

## 4. Cost estimate (not spent)

OpenAI standard-tier list prices read from developers.openai.com/api/docs/pricing on 2026-11-13, USD per 1M tokens (input,
output): gpt-4o-mini 0.15, 0.60; gpt-4o 2.50, 10.00; gpt-5 1.25, 10.00; text-embedding-3-small 0.02. The page lists the
`gpt-4o-mini-2024-07-18` snapshot only under fine-tuning, so the snapshot used by LOCOMO.md is priced at the gpt-4o-mini
standard rate. The page does not say how gpt-5 reasoning tokens are billed; they are assumed billed as output. No cache hits are
assumed, so each figure is an upper bound for the warm-cache case. `cost.py` in the evidence directory prints every line.

| Item | Arithmetic | USD |
|---|---|---|
| WARM held-out 7 (1,155 q), cap 1,764, 1 pass | reader 1,155 x 2,226 in + 6 out; judge 1,155 x 397 in + 12 out; gpt-4o-mini | 0.47 |
| same, 3 passes | x3 | 1.40 |
| WARM held-out 7, uncapped (3,354 in per reader call), 1 pass | | 0.66 |
| same, 3 passes | | 1.99 |
| mem0ai full run, 1 pass | 6,022 adds x 8,100 in + 200 out, plus 1,540 reader and judge calls | 8.68 |
| mem0ai, 3 passes | x3 | 26.03 |
| Graphiti full run, 1 pass | 5,882 episodes x 4 to 7 calls x 2.0k to 2.5k in; cross-encoder one call per candidate; 1,540 reader and judge | 10.80 to 21.50 |
| Graphiti, 3 passes | x3 | 32.41 to 64.50 |
| WARM on Mem0's harness, LoCoMo, gpt-4o, top-200 only | 1,540 x (9,055 in + 700 out) answerer + 1,540 x (853 in + 60 out) judge | 49.85 |
| same, default cutoffs 10, 20, 50, 200 | four answer and judge pairs per question | 123.98 |
| same, gpt-5 (2,000 reasoning plus output tokens answerer, 500 judge, assumed), top-200 only | | 57.57 |
| same, gpt-5, default cutoffs | | 192.58 |
| LongMemEval 500 q, gpt-4o / gpt-5 | embeddings 57.5M tokens (115k per question, assumed) plus answer and judge | 17.52 / 19.94 |
| BEAM 1M, 700 q, gpt-4o / gpt-5 | embeddings 35M tokens plus answer and judge | 23.62 / 27.00 |
| BEAM 10M, 200 q, gpt-4o / gpt-5 | embeddings 100M tokens plus answer and judge | 8.55 / 9.51 |

The LongMemEval and BEAM lines reuse the LoCoMo per-question token model, because their prompts and judges were not measured.
Three runs of any line cost three times that line, except that ingestion embeddings are paid once if the store is reused.

## 5. Reproduce

```sh
# free: development-set probe is in the evidence directory (cache-only embeddings, no key)
env -i HOME=$HOME PATH=$PATH uv run --no-project --with pytest --with tiktoken==0.12.0 --with openai==1.109.1 \
  --with nltk==3.9.1 --with jinja2==3.1.6 python -m pytest -q benchmarks/public/locomo
# stub plumbing for mem0ai (adapters must run under env -i)
env -i HOME=$HOME PATH=$PATH OPENAI_API_KEY=sk-stub uv run --no-project --with 'mem0ai[nlp,extras]==2.2.1' \
  --with openai==1.109.1 --with nltk==3.9.1 --with tiktoken==0.12.0 --with jinja2==3.1.6 python benchmarks/public/locomo/locomo_bench.py ...
```
