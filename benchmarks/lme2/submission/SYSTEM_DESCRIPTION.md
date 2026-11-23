# WARM G1: controller-selected state spans with verification (LongMemEval-V2, small tier)

Method: `warm_g1_live` (WARM, WritersLogic Adaptive Retention & Memory, Apache-2.0). Code: the single file `warm_g1_live.py` in this package.
Tier: small. Three operating points, each one live run of both domains with the same frozen design; they differ only in
the verify-and-reread step (step 6 below):

| point | step 6 | accuracy % [95% CI] | mean memory.query s |
|---|---|---|---|
| `op2` | off (`reread: false`) | 78.0 [74.3, 81.8] | 31.6 |
| `op1b` | on, skipped when the final verdict is absent (`skip_draft_when_absent: true`) | 78.9 [75.2, 82.7] | 66.3 |
| `op1` | on for every question (`reread: true`) | 77.2 [73.2, 81.2] | 119.1 |

Intervals are 4,000-resample bootstrap 95% intervals over questions.

## What the memory module does

`insert(trajectory)` stores the trajectory, ingests every state into a WARM engine store (form state parsed from the
accessibility-tree text), and builds a compiled index of the inserted haystack for the controller (page regions,
transitions, episodes, page inventories, a per-step value log, and per-trajectory notes).

`query(question, image)` runs, for that question only:

1. Controller (Gemini): at most 14 tool rounds over the compiled index; it submits at most 20 raw trajectory states, a
   support analysis and a support verdict (supported, partial or absent).
2. VERIFY, CRUX and, for questions that offer explicit choices (a fixed regex on the question text), OPTCHECK: one
   forced function call each over the full text of the selected states, run in parallel. Quotes are checked
   mechanically against the cited state text.
3. Final rule (deterministic): the verdict becomes absent when the controller says absent, VERIFY finds the premise
   violated, or CRUX reports the asked item missing.
4. Second controller pass (Gemini), only for choice questions where OPTCHECK found no option supported by a grounded quote:
   the controller gets the undecided options; its spans go first, then the first-pass spans, at most 20 states.
5. Renderer: support analysis and verdict, the list of spans, then each state as the upstream `codex` renderer prints it
   (page text and screenshot), with a form-control table from the WARM engine (`experience_form_tables`) before each
   state's page text; a fixed 140,000-token budget filled in controller order.
6. Verify-and-reread (`op1`; `op1b` except on absent verdicts; not `op2`): the module calls the reader (the same Qwen3.5-9B endpoint and settings
   the harness uses) on exactly the messages the harness would build from that context, extracts the draft answer,
   and runs ANSWER-CHECK (Gemini) against verbatim state text. When the draft is contradicted with at least one
   grounded quote and the verdict is not absent, the returned context starts with an Evidence Check block listing
   those quotes; otherwise the step-5 context is returned unchanged. On an absent verdict the Evidence Check block is
   never added, so `op1b` skips the draft there and returns the same context `op1` would, without the draft's latency.

Inputs at query time are the question text and image and the inserted trajectories. The module reads no question ids,
answers, labels, question types or cached model outputs. One insert-time artifact is loaded from disk: the per-trajectory
controller notes (one procedure note and one hint note per trajectory), written once by Qwen3.5-9B from each trajectory's
goal, outcome, URLs, thoughts and actions with the upstream note-generation prompt (`qwen_v6_retrieval_safe`; 3 of 200
trajectories fell back to a mechanical note). The same notes file was used throughout development (sha256
`743205b66dc394c2bc9eb138294d6cfc2a94b6905895a10f0818b90041d2c11c`). Building the controller index runs in a background
thread during insert; a query that starts before the index of the full haystack is built waits for it, and that wait is
inside its measured time.

## Models and settings

| role | model | where | settings |
|---|---|---|---|
| controller, pass 2 | `gemini-3.6-flash` | Vertex AI, global endpoint | thinking level MEDIUM, max 16,000 output tokens per turn, at most 14 tool rounds |
| VERIFY, CRUX, OPTCHECK, ANSWER-CHECK | `gemini-3.6-flash` | Vertex AI, global endpoint | thinking level MEDIUM, max 8,000 output tokens, one forced function call (retried once if none) |
| reader (harness) and draft (inside `memory.query`, `op1` and `op1b`) | `Qwen/Qwen3.5-9B` | vLLM 0.33.0 on the same VM | bf16, max-model-len 150,000, MTP speculative decoding (2 tokens), temperature 0.6, top-p 0.95, top-k 20, thinking on, max 20,000 completion tokens, memory context capped at 200,000 tokens |
| controller notes (insert-time, written once) | `Qwen/Qwen3.5-9B` | vLLM | temperature 0.6, top-p 0.95, top-k 20, thinking on, text only |
| judge | `gpt-5.2` | OpenAI API | harness defaults: reasoning effort medium, max 4,096 completion tokens |
| form tables | WARM engine `warm-python` 0.6.8 | built from writerslogic/warm commit d096063f | `experience_form_state=True` |

## Hardware and run setup

- One Google Cloud `a2-ultragpu-1g` VM (1x NVIDIA A100-SXM4 80 GB, 12 vCPU, 167 GB RAM), us-central1, flex-start
  provisioning. The vLLM reader server, the upstream harness and the memory module ran on this VM. Gemini calls go to
  the Vertex AI API over the network.
- Upstream harness (github.com/xiaowu0162/longmemeval-v2 at 2cc8c540), unmodified, run per domain with
  `--prompt-build-max-workers 4` (shared haystack, parallel prompt building) and `--reader-max-concurrent-requests 16`.
  The web and enterprise runs of one operating point ran at the same time and shared the one reader server, so up to
  8 queries were in flight together. `memory_query_duration_seconds` is the harness's own timer and includes this
  contention, all Gemini round trips and, for `op1` and `op1b`, the reader draft.
- The operating points ran on different days: `op1` on 2026-11-17 and 2026-11-21 (paused for Vertex 429s in between;
  a paused run starts no new query, and finished queries keep their recorded time), `op1b` on 2026-11-21, `op2` on
  2026-11-22. Gemini latency varies by day: in `op1` the controller averaged 49.8 s per question (calls 5.5 to 13.6 s
  on 2026-11-17), in `op1b` 24.2 s (calls about 2.2 s). Each run started only after a capacity probe of 40 recorded
  requests showed no 429s and a controller-call p90 under the gate.
- Code file: `warm_g1_live.py` in this package (sha256 `d62948eb3f7a40b038e44351652270adbd8b1419aab0d31402cc9c77bd7e9869`) ran `op2`. `op1` ran sha256 prefix `dbca3f9f` and
  `op1b` `2e45588e`; the differences are the `skip_draft_when_absent` parameter (default false, which is `op1`) and,
  for `op2`, Gemini requests sent over persistent HTTPS connections instead of a new connection per call (the same
  request bytes, URL, headers that matter, retries and errors). The embedded stage sources are identical in all three.
- Driver additions outside the harness code (`live_harness.py`, not part of the scored module): the reader stage is the
  harness's own request with up to four attempts on transient errors (the harness itself aborts on the first one); judge
  requests are forwarded unchanged to the OpenAI API through a relay on the operator's workstation so the API key never
  sits on the VM; each harness run is otherwise the stock `evaluation.harness.main`.

## Cost per question (measured)

`op2`: $0.1684 Gemini per question (6,238 calls; 53% of prompt tokens were reported as implicit-cache hits).

`op1b`: $0.1806 Gemini per question (6,445 calls). `op1`: $0.1852 (6,580 calls). Gemini cost is computed at list
price from the reported token counts, without implicit-cache discounts. The judge (gpt-5.2) cost about $0.0013 per question.

## Selection history (plain statement)

- The design was developed on the LongMemEval-V2 small tier itself, split by question into a dev half (228 questions)
  and a held-out half (223 questions). There is no separate test set.
- Phases 4 to 7 used the dev half for every design choice (controller workflow, stages, rules, budgets, renderer),
  scored with a Gemini proxy judge; mechanisms that did not help on dev were dropped. The design was frozen on
  2026-11-17 (FREEZE6) before any held-out stage output existed.
- The held-out half was scored once, with gpt-5.2, from a staged run (cached stage outputs, reader batched offline):
  73.5 [67.7, 79.4]. Staged dev with gpt-5.2: 81.6; staged all 451: 77.6 [73.8, 81.4]. Those staged numbers are not
  submission numbers: their latency was not measured by the harness.
- Each operating point is one live run of the frozen design over all 451 questions. Because the dev half was used for
  selection, all-451 accuracy is optimistic relative to unseen questions. On the held-out half alone: `op1` 76.7,
  `op1b` 77.1, `op2` 76.7 (staged held-out 73.5).
- After the freeze, only the operating-point switches above and the transport changed. `skip_draft_when_absent` and
  `op2` were chosen from latency traces, not from scores: `op2` uses the no-reread path measured on dev in phase 7
  (reread +0.9 on dev, net +2 of 228), and `op1b` returns the same contexts as `op1` by construction.
- No prompt, rule, threshold or model was changed after the freeze. The live module was checked against the staged
  pipeline before each run: on 48 dev questions, with model outputs replayed from the staged caches, every Gemini request
  body (743), every draft reader request and every final reader request were byte-identical. For `op2` the same
  replay checks that no draft or ANSWER-CHECK request is made and that the final reader request equals the staged
  pass-1 request (695 Gemini requests, 0 mismatches).

## Known differences from the staged development runs

- In the staged runs, questions where the reread did not fire kept the checked draft as the final answer. In the
  harness the reader is called again on the returned context, so the final answer is a fresh sample at temperature 0.6
  rather than the draft ANSWER-CHECK examined.
- All model calls are live and sampled, so controller selections, stage verdicts and reader answers differ run to run.
