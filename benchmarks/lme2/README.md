# LongMemEval-V2 adapter: design G1

G1 is a memory module for the LongMemEval-V2 benchmark (LME-V2). LME-V2 asks questions about customized web and enterprise applications, recorded as agent trajectories. The answering model is a fixed reader (Qwen3.5-9B). G1 decides which raw states the reader sees and what it is told about them. The pipeline runs in this order:

1. A Gemini controller searches a compiled index of the trajectories and selects at most 20 states. It also writes a support analysis and a verdict (SPAN_V).
2. VERIFY and CRUX checks, plus a deterministic rule (c1), turn the verdict into "absent" when the question's premise is false.
3. Questions that offer explicit choices get an option check. When no option is decided by a grounded quote, a second controller pass runs.
4. The renderer puts a form-control table before each state's page text and fits the context into a 140000-token budget. The table comes from the WARM engine's experience form state.
5. After the first reader answer, an answer check compares that answer against verbatim state text. When the answer is contradicted, the reader is asked once more, with the contradicting quotes placed first.

Every prompt, schema, model, budget and ordering rule matches the frozen design (FREEZE6, 2026-11-17). `check_frozen.py` asserts this.

## Prerequisites

- **Upstream harness:** clone https://github.com/xiaowu0162/longmemeval-v2 at commit `2cc8c540bdb87fe6761629b585e727e1c4704520` and point `LME2_UPSTREAM` at it.
- **Dataset:** the LME-V2 data from https://huggingface.co/datasets/xiaowu0162/longmemeval-v2, prepared as described in the upstream `data/README.md`. Point `LME2_DATA` at the directory that holds:
  - `questions.jsonl`
  - `haystacks/lme_v2_small.json`
  - `trajectories.jsonl`
  - `screenshots/<trajectory_id>/<step>.png`, extracted from the trajectory screenshot archives

  The small-tier trajectories are cached on first use at `$LME2_TRAJ_CACHE` (default `$LME2_DATA/lme2_small_trajectories.pkl`).
- **Python 3.12 packages:** the upstream requirements (`huggingface_hub numpy openai openai-agents pillow tqdm transformers`).
- **WARM wheel:** build it from this repository with `maturin build --release` and install the wheel it writes under `target/wheels/`. The module is `writerslogic_warm`. It is needed only for `--form-backend engine`, which is the default.
- **Reader:** vLLM 0.33.0 serving Qwen3.5-9B in bf16 on one GPU. `serve_reader.sh` has the exact frozen flags:
  ```
  --dtype bfloat16 --max-model-len 150000 --gpu-memory-utilization 0.92
  --max-num-batched-tokens 16384 --max-num-seqs 32 --reasoning-parser qwen3
  --limit-mm-per-prompt '{"image":24}' --speculative-config '{"method":"mtp","num_speculative_tokens":2}'
  ```
  It also sets `VLLM_USE_FLASHINFER_SAMPLER=0`.

  Reader sampling is fixed in `reader_stage.py`:
  - temperature 0.6, top_p 0.95, top_k 20
  - thinking on
  - at most 20000 completion tokens
- **Controller notes:** `LME2_NOTES` points at the controller notes, which hold one procedure note and one hint note per small-tier trajectory.
  - `notes_stage.py --ids <json list of small-haystack trajectory ids> --out notes.json` writes them with the reader.
  - The frozen run used one sampled notes file (sha256 `743205b66dc394c2bc9eb138294d6cfc2a94b6905895a10f0818b90041d2c11c`). The notes are sampled at temperature 0.6, so regenerating them gives the controller different notes.

### Environment variables

Endpoints and keys come only from the environment.

| Variable | Used by | Meaning |
|---|---|---|
| `LME2_DATA`, `LME2_UPSTREAM` | all | dataset dir, upstream checkout |
| `LME2_NOTES` | `span_stage.py`, `ctl_pass2.py` | controller notes json |
| `LME2_TRAJ_CACHE` | all | optional trajectory cache path |
| `LME2_VERTEX_PROJECT` | Gemini stages | Google Cloud project serving Gemini on Vertex AI (global endpoint) |
| `LME2_VERTEX_TOKEN` | Gemini stages | optional OAuth token; otherwise `gcloud auth print-access-token` |
| `READER_BASE_URL`, `READER_API_KEY` | reader, notes | vLLM OpenAI-compatible endpoint (default `http://localhost:8023/v1`), key (default `EMPTY`) |
| `OPENAI_API_KEY` | `run_lme2.py full` | judge key (gpt-5.2); pass it only to the judge process |
| `LME2_FORM_BACKEND` | `run_lme2.py` | `engine` (default) or `reference` (formstate.py in Python, for testing) |

## Reproduction

Run these from the repository root with the variables above set. `B=benchmarks/lme2`, `RUN=runs/g1`, `mkdir -p $RUN`.

1. **Pass-1 controller** (Gemini):
   ```
   python $B/span_stage.py --ids $B/held_ids.json --out $RUN/span_v.jsonl --workers 10
   ```
   - Model `gemini-3.6-flash`, workflow_doc4, at most 14 rounds, MEDIUM thinking, at most 20 states.
2. **VERIFY** (Gemini):
   ```
   python $B/check_stages.py verify --spans $RUN/span_v.jsonl --out $RUN/verify.jsonl --workers 10
   ```
3. **CRUX** (Gemini):
   ```
   python $B/check_stages.py crux --spans $RUN/span_v.jsonl --out $RUN/crux.jsonl --workers 10
   ```
4. **Final rule c1:**
   ```
   python $B/rules.py final-rule $RUN/span_v.jsonl --verify $RUN/verify.jsonl --crux $RUN/crux.jsonl --out $RUN/spans_f1.json
   ```
5. **Choice ids:**
   ```
   python $B/rules.py choice-ids $B/held_ids.json $RUN/choice_ids.json
   ```
6. **OPTCHECK** (Gemini), over the pass-1 spans:
   ```
   python $B/check_stages.py optcheck --spans $RUN/span_v.jsonl --ids $RUN/choice_ids.json --out $RUN/opt.jsonl --workers 10
   ```
7. **Undecided ids:**
   ```
   python $B/rules.py undecided-ids $RUN/opt.jsonl $RUN/undecided_ids.json
   ```
8. **Pass-2 controller** (Gemini):
   ```
   python $B/ctl_pass2.py --ids $RUN/undecided_ids.json --opt $RUN/opt.jsonl --spans $RUN/span_v.jsonl --out $RUN/pass2.jsonl
   ```
9. **Merge:**
   ```
   python $B/rules.py mkp6 --base $RUN/spans_f1.json --pass2 $RUN/pass2.jsonl --out-spans $RUN/spans_s2.json
   ```
   - Pass-2 spans replace f1 spans. The f1 support verdict is kept.
   - The f1 analysis is kept when f1 says absent.
10. **Reader pass-1 rows:**
    ```
    for d in web enterprise; do python $B/run_lme2.py prompts --domain $d --ids-file $B/held_ids.json --spans $RUN/spans_s2.json --out $RUN/rows1_$d; done
    ```
11. **Reader pass 1** (GPU):
    ```
    python $B/reader_stage.py --rows $RUN/rows1_web/prompt_rows.jsonl $RUN/rows1_enterprise/prompt_rows.jsonl --cache $RUN/reader1.json
    ```
12. **ANSWER-CHECK** (Gemini), over the pass-1 spans:
    ```
    python $B/rules.py answers $RUN/reader1.json $RUN/answers1.jsonl
    python $B/check_stages.py anscheck --spans $RUN/span_v.jsonl --answers $RUN/answers1.jsonl --out $RUN/anscheck.jsonl --workers 10
    ```
13. **Reread rows:**
    ```
    python $B/rules.py mkp6 --base $RUN/spans_f1.json --anscheck $RUN/anscheck.jsonl --out-first $RUN/reread_first.json
    for d in web enterprise; do python $B/run_lme2.py prompts --domain $d --ids-file $B/held_ids.json --spans $RUN/spans_s2.json --first $RUN/reread_first.json --out $RUN/rows2_$d; done
    ```
    - A question fires when ANSWER-CHECK says contradicted with at least one grounded quote and the f1 verdict is not absent.
    - For those questions the Evidence Check block goes first. Every other question keeps its pass-1 rows.
14. **Reader pass 2** (GPU), only for rows whose messages changed:
    ```
    python $B/rows.py pending --rows $RUN/rows2_*/prompt_rows.jsonl --have $RUN/reader1.json --out $RUN/todo2.jsonl
    python $B/reader_stage.py --rows $RUN/todo2.jsonl --cache $RUN/reader2.json
    python $B/rows.py assemble --rows $RUN/rows2_*/prompt_rows.jsonl --have $RUN/reader1.json $RUN/reader2.json --out $RUN/rc_g1.json
    ```
15. **Judge** (OpenAI gpt-5.2 through the upstream evaluation, run once):
    ```
    for d in web enterprise; do env -i HOME="$HOME" PATH="$PATH" PYTHONHASHSEED=0 LME2_DATA="$LME2_DATA" LME2_UPSTREAM="$LME2_UPSTREAM" OPENAI_API_KEY="$OPENAI_API_KEY" python $B/run_lme2.py full --domain $d --ids-file $B/held_ids.json --spans $RUN/spans_s2.json --first $RUN/reread_first.json --out $RUN/judged_$d --reader-cache $RUN/rc_g1.json --judge-cache $RUN/judge_cache.jsonl; done
    ```
    - The judge calls are cached, and a spend ledger caps them (`--judge-cap-usd`, default 60).
    - The frozen run also passed `ARCH_TRUST_CACHE=1`. Its equivalent here is `--trust-cache`, which accepts a pinned reader output whose rebuilt context hash differs. Without it, a mismatch is an error.
16. **Report:**
    ```
    python $B/report.py --g1 "$RUN/judged_{}/per_question.jsonl" --base "<SPAN_V judged dir>_{}/per_question.jsonl" --stages-dir $RUN
    ```
    - The report gives bootstrap 95% intervals (4000 resamples, seed 7), paired differences, latency and Gemini cost.
    - This adapter does not render the SPAN_V baseline column. That column comes from the frozen phase-3 SPAN_V run, judged the same way.

## Result (held-out, measured)

These are the 223 held-out questions (`held_ids.json`), with the gpt-5.2 judge run once. Each cell is the score with its 95% bootstrap interval. The last column is the paired G1 minus SPAN_V difference.

| subset | n | G1 | SPAN_V | paired difference |
|---|---|---|---|---|
| overall | 223 | 73.5 [67.7, 79.4] | 66.8 | +6.7 [+2.7, +10.8] |
| answerable | 160 | 71.2 [64.4, 78.1] | 68.1 | +3.1 [+0.0, +6.9] |
| abstention | 63 | 79.4 [69.8, 88.9] | 63.5 | +15.9 [+4.8, +27.0] |
| web | 118 | 86.4 [80.5, 92.4] | 78.8 | +7.6 [+2.5, +13.6] |
| enterprise | 105 | 59.0 [49.5, 68.6] | 53.3 | +5.7 [+0.0, +11.4] |

- G1 won 18 questions and lost 3 against SPAN_V.
- The published reference figure of 74.9 lies inside the G1 interval. Against the published reference this is a tie within noise, not a win.
- The 228-question dev half was used for all design selection, with gemini-3.5-flash as a proxy judge. Only this held-out row is an unbiased estimate.
- `memory.query` latency before the reader: median 42.7 s, p95 132.6 s.
- Latency including the answer check: median 48.0 s, p95 137.0 s.
- Gemini cost: $0.1893 per question.

## Leaderboard submission (self-reported)

`submission/` is the record of the small-tier package submitted to the LME-V2 maintainers on 2026-11-22 (`warm_g1.tar.gz`, sha256 in `submission/warm_g1.tar.gz.sha256`). It holds the package's `SYSTEM_DESCRIPTION.md`, the single-file module `warm_g1_live.py`, `submission_overview.json` and each operating point's metric files. The per-question run outputs in the package are not committed.

- Three operating points, each one live run of all 451 small-tier questions (240 web, 211 enterprise): `op2` 78.0, `op1b` 78.9, `op1` 77.2 (accuracy %, judged once).
- LAFS 60.84 against the reference frontier's 55.76 (+5.07), computed with the upstream LAFS definition (small tier, 1 to 200 s).
- These full-set numbers include the 228 dev questions used for design selection, so they are optimistic. The held-out row above is the unbiased estimate.
- The maintainers have not verified these figures; they are self-reported.

## What runs without hosted models

The frozen run's stage outputs (controller spans, VERIFY, CRUX, OPTCHECK, pass-2 and ANSWER-CHECK results, reader answers), the controller notes and the original research scripts are not in this repository. From the public repository alone, what reproduces without hosted models is the engine form table: `tests/experience_form_state.rs` checks that it equals the reference renderer's output on 28 recorded states of the public trajectories. Everything else below needs those outputs, which a run of the hosted stages produces, and `check_frozen.py` and `smoke_test.py` need the reference files of the frozen run (`--reference-dir`), which are not published.

**Offline**, given the stage outputs of a run:
- the WARM engine form table and the rest of the renderer (`run_lme2.py prompts`)
- the deterministic stages (`rules.py`: final rule, choice ids, undecided ids, merge, reread rows, pass-1 answers)
- `rows.py` and `report.py`

**Gemini on Vertex AI:**
- the controller (`span_stage.py`)
- VERIFY, CRUX, OPTCHECK, ANSWER-CHECK (`check_stages.py`)
- pass 2 (`ctl_pass2.py`)

**GPU:** the reader (`reader_stage.py`, `notes_stage.py`).

**OpenAI:** the judge.

## Frozen prompts

- `prompts.py` holds every frozen string, schema and setting, extracted mechanically from the reference files. All stages import it. It contains:
  - the controller workflow (`WORKFLOW`)
  - the compiled-memory and span tool specs
  - the VERIFY, CRUX, OPTCHECK and ANSWER-CHECK system prompts and function schemas
  - the user-message templates and the request body
  - the pass-2 memory note
  - the Evidence Check text
  - the support-verdict texts
  - the form-control block
  - the choice regex
  - the budgets and caps
- `ctl_g.py` keeps the controller loop's in-loop messages unchanged: the progress note, rejection and continue prompts.
- `envmodel.py`, `tools2.py` and `tools3.py` hold the controller tools. `envmodel.py` and `formstate.py` are byte copies apart from the license header.

### Checks

`check_frozen.py --reference-dir <reference files>` compares the port against the original files. It parses them and never imports them:
- every prompt and schema
- each message template, evaluated on sentinel inputs
- the controller loop
- the tool classes
- the render and grounding functions
- the reader, judge and serve settings

`smoke_test.py --reference-dir <reference files>` checks the deterministic stages and the renderer (with `--form-backend reference`) against the frozen run's outputs. It also checks the engine input bounds, and that a form table computed from the tree dicts alone equals formstate's table for every state. It needs no network.

## Engine contract this adapter relies on

The form backend is in `formtable.py`.

- **Source of the controls:** the engine parses the form controls from the `accessibilityTree` text. The tree dicts carry everything formstate reads; `smoke_test.py` checks this on all 5095 small-tier states.
- **Acceptance check:** after `maturin build`, run `smoke_test.py --engine`. It requires the engine backend to render the same items as the reference backend.

- **Ingest:** each trajectory is ingested once, in state order, at `Memory.insert`, into a fresh store directory per memory instance. Each state is one `w.remember({"episode", "step", "tree", "url", "action"?, "accessibilityTree"})` call.
- **Steps:** `step` is the state index.
- **Action:** `action` is omitted for the first state and for empty actions.
- **Tree:**
  - It is built from the accessibility-tree text. Each line is a node with an optional `[handle]` (`\w+`, so alphanumeric handles such as `a319` are kept), a role, an optional quoted name, and the rest of the line as `props`.
  - A line nests under the nearest earlier line with less leading whitespace. Several top-level lines get a synthetic `RootWebArea` parent.
- **Bounds:** strings are clipped to 16384 bytes, page text to 4 MiB, at most 200000 nodes, and depth stays below 128.
- **Form tables:** `w.experience_form_tables([(trajectory_id, state_index), ...])` is called once per query with every state of the valid spans in render order, duplicates included. It must return one block per state, with the "already shown" set running across the list.

## Evidence-lifecycle fixes (not yet scored)

`evidence_v2.py` holds fixes found in a source review. The frozen G1 files are unchanged, so `check_frozen.py` still pins them, and nothing here has a scored run.

- `Tools4`: indexes the whole page in 20,000-character windows (G1 indexes only the first 20,000 characters; 42% of the small-tier states are longer and 46% of all page text lies beyond that cap), `show_state` takes an `offset`, `find_states` reports page length, and `value_timeline` lists every logged observation of one control in state order.
- `normalize_spans`, `merge_pass2_v2`: count unique (trajectory, state) pairs against the 20-state allowance; G1 deduplicates only exact span dictionaries.
- `final_rule_v2`, `apply_pass2_v2`: G1's `absent` verdict never changes once set, even when pass 2 or VERIFY and CRUX find the evidence. These record the basis of an `absent` and revise it when only the controller held it.
- `test_evidence_v2.py` checks these invariants without models or labels.

Protocol notes. The official harness calls the reader once on the `memory.query` output. G1's ANSWER-CHECK reads the first reader answer and can rerun the reader outside the harness, so a G1 number that includes that step is not a reference-protocol number; a compliant run does that work inside `memory.query` and counts its latency. `run_lme2.py --trust-cache` accepts a reader output whose rebuilt context hash differs; do not use it for a headline reproduction.
