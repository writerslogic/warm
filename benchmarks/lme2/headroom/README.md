# V2 failure classification and correspondence headroom (development only)

Starting commit: `1bf2cbe9` on `feat/experience-procedures` (PR #509). Judge-free; no model was called; held-out questions, answers, judgments and failure lists were not opened. The development allowlist is every id in `questions.jsonl` that is not in `../held_ids.json` (228 of 451; the 223 held ids are dropped at load). `classify.py` asserts the two sets are disjoint and that the failure file covers exactly the 228.

Reproduce: `uv run python benchmarks/lme2/headroom/classify.py --root /Volumes/A/.hms-lb/lme2` (reads `review.json`, rewrites `classification.json`) and `uv run python benchmarks/lme2/headroom/evidence_audit.py --root /Volumes/A/.hms-lb/lme2` (rewrites `evidence_audit.json`). Decision rules: `DECISION_CRITERIA.md`.

## Population

Round-6 system on the 228 development questions: 97 failures (`final-arch/forensics_r6_dev.json`, gpt-5.2 judge for abstention and gotcha questions, string match otherwise). Round 6 is not a zero-LLM memory: the reader model writes notes and a support analysis for memory (see `../RUNS.md`). This inventory is the failure distribution of round 6 only. It is not the distribution of G1, whose per-question development judgments were not located, nor of the zero-LLM baseline (round 5 `slice12i`, dev 48.68, which is where the 48.7 figure comes from), whose failures were not classified.

## Classification (97 failures; a failure can have contributing causes)

| Category | Primary | Primary or contributing |
|---|---:|---:|
| 3 missing retrieval candidates | 16 | 22 |
| 5 temporal scope / state tracking | 0 | 22 |
| 6 incomplete multi-source evidence | 0 | 17 |
| 7 failure-recovery / procedural advice | 25 | 25 |
| 9 unsupported absence / false premise | 25 | 25 |
| 10 reader interpretation | 22 | 26 |
| 12 mixed or unclassifiable | 9 | 24 |
| 4 cross-observation entity correspondence | 0 | 0 |
| 1, 2, 8, 11 | 0 | 0 |

Each failure in `classification.json` records how its labels were made:

- `cause_basis: heuristic`: the cause labels come from rules over question type, a regular expression over the question text, and the recorded gold-phrase flag.
- `correspondence_basis: manual`: the correspondence judgement was made by reading the question. Its rationale is in `review.json`, as one of six reviewer-assigned kinds per question.
- `counterfactual` and `identity_oracle_constructible`: `null` means not run or unknown. It is not false.
- `gold_phrase_in_context: null`: not checkable. It is different from false.

No label in this file is source-supported or counterfactual. The only counterfactual artifact is the earlier evidence-oracle run, audited separately in `evidence_audit.json`.

How each label was assigned: abstention types (`*-abs`) are 9; `errors-gotchas` and `procedure` are 7; the rest are 10 when the gold phrase is in the final context, 3 when it is not, and 12 when no phrase check applies (multiple choice). Category 5 is added to dynamic-environment questions and 6 to questions that compare pages, by reading the question text. Phrase presence is a proxy: it does not show the evidence was complete or bound to the right control, so categories 3 and 10 are suspected, not established. Gold phrase in context: 26 true, 22 false, 49 not checkable. Categories 1, 2, 8 and 11 were not assigned because nothing in the available artifacts separates them from 3 and 10; "0" there means not measured, not absent.

## Entity correspondence

No correspondence case was identified in the reviewed round-6 failure population, and no identity counterfactual was run. The review read the question and gold answer of all 97 failures, not the trajectories. None was judged to need the same record, row or control matched across observations without an exact identifier. The per-question rationale is in `review.json`. The dataset has no entity-identity or evidence annotations (the entry README: "no gold evidence-trajectory labels"), and no oracle was built to rescue the direction. Under `DECISION_CRITERIA.md`, correspondence is deprioritised for this workload. This is a judgement about these questions read from their text. It is not a measured absence of headroom, and it says nothing about interfaces where records move and repeat.

## Original claims, checked against the artifacts

- "I1 programs helped 1 of 13": confirmed as stated and corrected in meaning. I1 was an LLM-written Python program run over the compiled memory on 13 dev web questions with a number or ordered gold; 1 of 13 correct against a 40% kill line (`arch/DESIGN.md` line 132). It does not test learned abstractions or entity correspondence.
- "44% rare-term coverage": confirmed. It is the median fraction of rare gold terms present in the 8k pack over 15 dev gotcha questions (`final-arch/a2_run1.txt`), with about 89% present in the haystack. It is term coverage, not complete evidence recall or answer correctness, and n = 15.
- Failure-recovery mining without a model (I5) was measured as rare-term coverage on 15 development gotcha questions: .33 for the mined rules against .44 for the 8k pack, and .48 when the rules were appended (+.04, within noise at n = 15). That tested one rule miner by term coverage. It did not test answers, and it does not rule out other recovery-note designs.
- The evidence-oracle run ("60 of 104 against 60", `arch/DEV_EVALS.md` row 4) is reproducible from its reader outputs (`evidence_audit.py`, output `evidence_audit.json`). Rescored locally, 106 phrase questions had oracle evidence, not 104: oracle 60, round 6 61 on the same 106. Paired with round 6: 44 retained, 16 recovered, 17 regressed, 29 unresolved. The oracle block was prepended to the `arch_uep16` pack, not to the round-6 context, so round 6 is not its control. Against `arch_uep16` without the block: 60 against 58, with 52 retained, 8 recovered, 6 regressed and 40 unresolved. The oracle selected units and transitions whose text contains a gold phrase, so it shows the phrase was present, not that the evidence was complete or bound correctly. On 106 phrase questions with this reader, prompt and pack, adding phrase-bearing units changed little. It is not a general evidence ceiling.

## Decision

No candidate has a measured judge-free gain that would justify building it now. No correspondence cases were identified in the reviewed failures. The one recovery-note miner tested did not raise term coverage. Adding phrase-bearing units to one pack moved 60 against 58 on 106 questions. Nothing further was added to the default path. The retained work is the occurrence-provenance repair and this classification. The largest remaining bucket is false-premise and abstention handling plus procedural advice (50 of 97), which the G1 stages (VERIFY, premise checks) already target and which need the official judge to measure.

## Official evaluation (not run)

Needs the gpt-5.2 judge, which is not authorised here. When it is: `run_lme2.py` stages from the entry README, reader Qwen3.5-9B bf16, all 451 questions per tier, live non-cached query timing, no `--trust-cache`. The reader's re-run inside G1's answer-check is outside the reference protocol and must be disabled for a compliant number.
