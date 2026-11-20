# V2 failure classification and correspondence headroom (development only)

Starting commit: `1bf2cbe9` on `feat/experience-procedures` (PR #509). Judge-free; no model was called; held-out questions, answers, judgments and failure lists were not opened. The development allowlist is every id in `questions.jsonl` that is not in `../held_ids.json` (228 of 451; the 223 held ids are dropped at load). `classify.py` asserts the two sets are disjoint and that the failure file covers exactly the 228.

Reproduce: `uv run python benchmarks/lme2/headroom/classify.py --root /Volumes/A/.hms-lb/lme2` (rewrites `classification.json`). Decision rules: `DECISION_CRITERIA.md`.

## Population

Round-6 system on the 228 development questions: 97 failures (`final-arch/forensics_r6_dev.json`, scored by the original round-6 evaluation; its judge provenance is in that run). This is not the G1 pipeline. G1's per-question development judgments were not located, and its judge was gpt-5.2, so G1 failures are not classified here. Round 6 held-out was 51.1 and dev 57.5; G1 measured dev 81.6 and held-out 73.5 under gpt-5.2 (`arch/DESIGN.md` section 10). The 48.7 figure quoted in the request was not found in the V2 notes or the repository and is not used.

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

How each label was assigned: abstention types (`*-abs`) are 9; `errors-gotchas` and `procedure` are 7; the rest are 10 when the gold phrase is in the final context, 3 when it is not, and 12 when no phrase check applies (multiple choice). Category 5 is added to dynamic-environment questions and 6 to questions that compare pages, by reading the question text. Phrase presence is a proxy: it does not show the evidence was complete or bound to the right control, so categories 3 and 10 are suspected, not established. Gold phrase in context: 26 true, 22 false, 49 not checkable. Categories 1, 2, 8 and 11 were not assigned because nothing in the available artifacts separates them from 3 and 10; "0" there means not measured, not absent.

## Entity correspondence: 0 of 97, and no oracle can be built

All 97 questions and gold answers were read. None requires matching the same record, row or control across observations without an exact identifier. They ask for the default value of a field, which buttons a page shows, how many stages a workflow has, or which of several forms has a property; comparing two forms is aggregation over pages, not identity. The dataset has no entity-identity or evidence annotations (`README.md` of the entry: "no gold evidence-trajectory labels"), so an identity oracle from independently grounded links cannot be constructed, and Phase 4 was not run. Under the decision rules (20 of 97 needed), correspondence is rejected as the main intervention for this workload. That says nothing about interfaces where records move and repeat; it says this question set does not test it.

## Original claims, checked against the artifacts

- "I1 programs helped 1 of 13": confirmed as stated and corrected in meaning. I1 was an LLM-written Python program run over the compiled memory on 13 dev web questions with a number or ordered gold; 1 of 13 correct against a 40% kill line (`arch/DESIGN.md` line 132). It does not test learned abstractions or entity correspondence.
- "44% rare-term coverage": confirmed. It is the median fraction of rare gold terms present in the 8k pack over 15 dev gotcha questions (`final-arch/a2_run1.txt`), with about 89% present in the haystack. It is term coverage, not complete evidence recall or answer correctness, and n = 15.
- Failure-recovery mining without a model (I5) was already measured: coverage .33 against .44 for the pack, +.04 when appended, within noise at n = 15. A deterministic recovery-note builder was therefore not rebuilt.
- With gold evidence supplied, the 9B reader did not exceed round 6 on answerable phrase questions: 60 of 104 against 60 (`arch/DEV_EVALS.md`, row 4). Evidence alone was not the only limit there.

## Decision

No candidate shows a measured judge-free gain: correspondence has no headroom on this population, recovery-note mining was killed by a recorded measurement, and the evidence-supplied ceiling did not rise. Nothing further was added to the default path. The retained work is the occurrence-provenance repair and this classification. The largest remaining bucket is false-premise and abstention handling plus procedural advice (50 of 97), which the G1 stages (VERIFY, premise checks) already target and which need the official judge to measure.

## Official evaluation (not run)

Needs the gpt-5.2 judge, which is not authorised here. When it is: `run_lme2.py` stages from the entry README, reader Qwen3.5-9B bf16, all 451 questions per tier, live non-cached query timing, no `--trust-cache`. The reader's re-run inside G1's answer-check is outside the reference protocol and must be disabled for a compliant number.
