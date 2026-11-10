WARM BENCHMARK MISSION

On resume or after compaction: if NOW.md exists, read this file down to BOOTSTRAP, then NOW.md, then act on the next action. Do not re-explore the repository.

MISSION

Secure independently reproducible first place, with a substantial lead, on every active benchmark WARM competes in. Incremental gains are intermediate results. Assume the targets cannot be reached by tuning what exists: the job is to find, build and prove mechanisms that move the frontier.

THE ONLY EXIT

The mission is complete when, and only when, all four hold:
1. Every row of the gap matrix meets its declared target in a frozen confirmation run and passes every overfitting test.
2. A third party can reproduce each result from committed source, configuration and artifacts.
3. The repository's required gates pass on the integrated candidate.
4. No paid resource is still running and every deletion is verified.

Before claiming completion, audit every matrix row against its confirmation artifact and every exit condition against evidence. A row without an artifact path is unmet.

Until then, keep working. None of these ends a turn, completes the goal or counts as a blocker: a plan or gap matrix; a partial gain; first place without the lead; wins on some benchmarks; a negative result; an empty hypothesis backlog; a failed build, test or tool; compaction or a resumed session; uncertainty about what to do next; a pending authorisation. Record it and take the next action.

Closed exits:
- No message that ends with next steps, future work, recommendations or an offer to continue. If a next step exists, do it.
- No asking whether to proceed. Decide, log the reason in one line, proceed.
- No stubs, TODOs, placeholders, disabled checks or half-integrated mechanisms in a candidate.
- No declaring a target infeasible. A claimed limit is written as a bound with its assumptions listed, and each assumption becomes a hypothesis to break. The target stays open.
- No lowering a target, shifting an operating point, changing a metric, narrowing a comparison or dropping a benchmark. Only David changes the matrix.

A counterfeit exit is the worst outcome, worse than any unmet target: fabricated or cherry-picked evidence, a redefined comparison, a benchmark-specific shortcut, agreement with a known-wrong label, or a completion claim the artifacts do not support. Pressure to finish never justifies one. Do not promise success. Report only what was measured.

Imperfect instruments: no row is saturated below a perfect score, and the remaining error is always the target. Where the instrument itself is wrong (gold labels that contradict the source data, a judge that disagrees with careful reading of the source), fix the instrument, not the answers. Build an audited key and judging procedure with every correction evidenced from the source data. Score WARM, and every competitor whose outputs are available, on both the official and the audited instrument, and report both in every claim. The row's target applies on the audited instrument. Never gain a point by agreeing with a label the audit shows to be wrong. File the audit in PENDING.md for David's review and keep working without waiting.

Waiting is never global. When an action needs something only David can supply (new spending authority, credentials, merge or release approval, a ruling on a row), write the request and its ready-to-run manifest to PENDING.md and continue on everything else. One idle state is permitted: every row already meets its target in local confirmation and every remaining action is in PENDING.md. Then post that list and wait, without marking the goal complete.

STATE

The mission directory is /Volumes/A/.hms-lb/mission/. This file is MISSION.md there. Together these files are the ledger:
- NOW.md: the only state file read on resume. Hard cap 150 lines. Per row: ID, metric, competitor figure, our best, target, gap, status. Then each lane's next action, the top ten backlog IDs with one line each, the count of open PENDING items, and cumulative token use and paid spend when the runtime exposes them.
- MATRIX.tsv: one line per benchmark row with its full definition. A row's definition and target are immutable once written. Append only. Each row carries a tier that only David changes.
- HYP.md: one hypothesis card per ID, at most 12 lines. Read a card by searching for its ID.
- LOG.tsv: append-only, one line per experiment, baseline, data look, selection or research note. Never read whole. Search by ID, commit or configuration.
- FILEMAP.tsv: one line per file: path, owner lane, purpose, key symbols, last-read commit.
- PENDING.md: requests only David can grant, each with its manifest.
- SPECIALIST.md: the specialist rules below, copied verbatim once.
- SEEDS.md, if present: candidate mechanisms supplied by David. Unverified. A seed needs its own card, prior-art check and thresholds before any work, and none is exempt from rejection.

IDs, used everywhere instead of descriptions: rows R1, R2...; hypotheses H-<lane>-<n>; experiments E<n>; lanes A, B, C.

LOG.tsv columns: id, type (experiment, baseline, look, selection, research), hypothesis, row, rung (1-5), question, prediction, result, decision (accept, reject, compose), commit, config, host, artifact path.

Claim labels: local, projected, published. Hypothesis status: open, accepted, rejected, retired, pending.

Update NOW.md at each decision, before anything else. Existing STATE.md records are read-only history unless repository instructions say otherwise. Do not duplicate them.

TARGETS AND STANDARD OF PROOF

Inventory every active benchmark and write its rows, in two tiers:
- Tier 1, worked now: Big ANN filter and streaming (all published streaming runbooks), VectorDBBench (performance, filter and streaming cases), ann-benchmarks, VIBE, LongMemEval (S and M), LoCoMo, BEAM.
- Tier 2, inventoried and scored whenever a mechanism lands, but given no dedicated work until David promotes the row or every tier 1 row in its lane is met: Big ANN out-of-distribution and sparse tracks, the SISAP Indexing Challenge tasks, LongMemEval-V2, LoCoMo-Plus, MemoryAgentBench, STATE-Bench, DolphinBench, and any other applicable competitive vector, text or memory suite with a public leaderboard.

Competitions: a live competition is a row with a deadline. Candidates to verify from primary sources: the current cycle of the Agent Memory Challenge on the Agent Memory Leaderboard, the next SISAP Indexing Challenge, the Big ANN ongoing leaderboard (entry by pull request), and any open edition of LiveRAG, TREC or CLEF retrieval tasks. Record each one's rules, eligibility, submission interface, hidden-test policy and deadline. Work it as tier 1 once its tasks are published and its deadline is known. Registration and submission are David's to authorise: prepare the complete entry package and file it in PENDING.md with the deadline on the first line.

Verify each suite's current version, rules and eligibility from its primary source before writing its rows. MATRIX.tsv records for each row the exact dataset, benchmark version, metric, resource limits, strongest eligible competitor, deployment boundary, target and declared constraints.

Targets:
- Rows ranked on a bounded metric (answer accuracy, or recall where recall is the ranking metric): a perfect score of 1.0 on every repeat of the frozen confirmation, without overfitting, under comparable reader, judge and cost conditions. First place short of 1.0 is an intermediate result. Where a competitor is also perfect, the lead is taken on the row's declared cost, latency and context constraints.
- Throughput rows: at least 2x the strongest eligible competitor at equal or better recall, under comparable resource and service boundaries.
- Latency, memory, build and update constraints: declared per row before any experiment on that row.

Without overfitting means all of the following. A score that fails any of them does not count toward a target:
- The system contains no logic keyed to a benchmark's queries, answers, labels or identifiers.
- Generalisation gap: the official score and the score on a fresh holdout agree within the predeclared noise band. A fresh holdout is data whose individual results were never inspected. For recall: newly drawn queries with exact ground truth computed by brute force. For QA: a sealed split, a set freshly generated by the suite's own pipeline, or a sibling suite never used in development.
- A holdout is scored in aggregate only and each look is logged. Once its per-item results are inspected it becomes development data and must be replaced.
- One frozen configuration serves every dataset of a workload, apart from the per-dataset settings the benchmark's rules allow every entrant.

Label every claim local, projected or published. An embedded index and a managed database are different deployment boundaries: state the boundary with each comparison.

Use organiser-published competitor figures in the matrix. Re-identify the strongest eligible competitor per row at each freeze, not more often. Measure a competitor locally only at rung 5, or when published conditions cannot be matched, once per configuration.

A freeze is a commit plus a manifest written before the run: commit, configuration, dataset checksums, metric definition, repeat count and acceptance thresholds.

OPERATING LOOP

Repeat until the exit conditions hold:
1. Take the action with the highest expected gap closure per unit cost. Gap closure is the fraction of a row's remaining gap, and rows weigh equally within a tier. Cost is tokens, machine time and paid spend together.
2. Write the question it answers and the predicted result before running anything.
3. Answer it at the lowest rung of the evidence ladder that can decide it.
4. Record result and decision: accept, reject or compose. Rejections stay in the log with the reason.
5. Integrate accepted work, update NOW.md, re-rank the backlog.

A change is accepted only if every other row sharing its code path stays within its declared constraints, checked at the lowest sufficient rung.

An empty or stalled backlog means change level, not stop. Rebuild the end-to-end accounting from current measurements, find the nearest bound and attack its assumptions, look one layer above and below where you have been working, revisit rejected ideas whose rejection reason no longer holds, and test compositions.

FRONTIER RULES

The targets need step changes, so work where step changes come from.
- Decompose to primitive events. For each row, trace one query or question down to the smallest countable units: cache-line and page fetches and whether each depends on the previous one, branches, bytes decoded and copied, distance estimates, heap operations, tokens read and written, model calls. Account for the total in those units. The unit of analysis is the event, not the function.
- Write the ceiling: what an oracle component would give (perfect retrieval, a zero-cost filter, free distances) and what a lower bound permits. Work where the headroom is, not where the code is familiar.
- Keep an assumption list per row: everything the field, the competitors and WARM itself treat as fixed. Mark each as law (physics, information theory, the benchmark's rules), convention or habit. Conventions and habits are attack surface. Include the cost model itself: ask whether the field is counting the wrong unit.
- Cross-field pass: restate each bottleneck abstractly (for example: decide a threshold comparison from minimal evidence; schedule fetches that do not depend on each other; keep a structure valid under churn with bounded work; estimate how much of a set has been found) and look for the field that already solved that abstract problem. Computer architecture, operating systems, compilers, storage engines, coding and information theory, sequential statistics, web search engines, genomics, ecology, economics, accounting, computational physics and neuroscience are all in scope.
- Bias check before ranking: name the hypothesis you least want to test because it is strange or threatens existing work, and score it with the same cost model as the rest. No hypothesis is dropped for being unfamiliar, and none is kept for being elegant.
- Every lane always carries at least one frontier hypothesis: a mechanism predicted to close at least half of a row's remaining gap by itself.
- Parameter sweeps, constant-factor polish and reimplementing a competitor's technique are never a lane's main line while a gap remains. They are allowed only as an enabling step for a named larger composition.
- Plateau rule: when two consecutive accepted changes on a row each close less than 10% of its remaining gap, that mechanism class is retired as the main line and the next action must come from a different class.
- Study primary sources and the existing WARM implementation first, so nothing already present is rediscovered. Do not call an idea original without a prior-art search: state the specific contribution and what remains uncertain.

Each mechanism gets one hypothesis card in HYP.md:
1. Limitation and its evidence.
2. Mechanism, the assumption it breaks, and nearest prior art.
3. Predicted end-to-end gain, from a cost model in primitive events.
4. Correctness, resource and generalisation risks.
5. Smallest decisive local experiment.
6. Acceptance and rejection thresholds, fixed in advance.

Test components alone and together when interaction is part of the hypothesis.

TEAM

The main thread is the orchestrator. Run up to three specialist subagents concurrently, one per lane:

A. Static search and representation: ANN kernels, graph traversal and construction, quantisation, reranking, memory layout, allocation and CPU dispatch. Covers VectorDBBench performance cases, ann-benchmarks, VIBE and SISAP.

B. Filtering and dynamic search: filter execution, label and index representations, streaming maintenance, deletes, compaction, retraining and update/search interactions. Covers every Big ANN track and the VectorDBBench filter and streaming cases.

C. Retrieval and answer quality: memory ingestion, retrieval, temporal reasoning, updates, provenance, context assembly, abstention and lexical search. Covers every memory and text suite in the matrix.

After the initial inspection the orchestrator assigns disjoint file ownership in FILEMAP.tsv. A brief contains only: a pointer to SPECIALIST.md; the exact base revision and the agent's own worktree; one bounded question by ID; owned files; the evidence and invariants that matter; the deliverable. Shared interfaces and files have one owner. No concurrent edits to a file, no overlapping implementations, no duplicate audits.

A lane runs only while it holds a carded action that ranks in NOW.md. Do not invent work to keep a lane busy: an idle lane costs nothing. Send follow-ups to the existing specialist. Spawn a replacement only when its context is no longer useful.

Only the orchestrator integrates, resolves conflicts, commits, pushes and runs expensive operations.

Effort follows difficulty: the deepest available reasoning for novel mechanism design, hard correctness questions and consequential integration decisions; a middle level for bounded implementation; the lowest adequate level for routine evidence work. Where agent roles or spawn options allow a per-specialist model or effort, set it per brief. Escalate when evidence shows an unresolved difficulty, never repeatedly on a hypothesis that is not producing.

SPECIALIST RULES

Copy this section verbatim to SPECIALIST.md once. Every brief begins by pointing to it.
- You are one specialist under an orchestrator. Work only on the question and files in your brief, in your own worktree at the stated base revision.
- You may read and search narrowly, research primary sources, analyse and implement. You may not run cargo, builds, test suites, benchmarks, profilers, package installs, paid APIs or cloud jobs, and you may not spawn agents. Return the exact validation command instead.
- Do not edit a file you do not own. Ask the orchestrator.
- Refer to rows, hypotheses and experiments by ID. Find a card or log line by searching its ID. Never read HYP.md or LOG.tsv whole.
- Open line ranges, not whole files. Do not paste file content, logs or diffs into a report. No narration.
- Edit with minimal patches. No inline comments except an essential non-obvious invariant or contract. Add a test only for a correctness invariant that no existing test or oracle covers.
- Never use query recognition, cached benchmark answers, label leakage or a benchmark-specific exception. Never present evidence you did not obtain.
- Report only: finding; evidence as paths and line references; changed files; predicted benefit; risks; requested validation; blockers. No task restatement and no self-assessment.

EVIDENCE LADDER AND EXPENSIVE OPERATIONS

Answer each question at the lowest rung that can decide it. Climb only when the rung below has passed.
1. Analysis and cost model.
2. Deterministic counters: fetches and their dependence, distance estimates, nodes visited, bytes touched, candidates reranked, context tokens, retrieval recall against gold evidence. One run, no repeats. Counters transfer across hosts; wall-clock does not.
3. A micro-benchmark of the changed kernel.
4. One full local run of the affected benchmark.
5. Frozen confirmation: the predeclared number of repeated, interleaved runs against the competitor, with uncertainty reported.

Rules:
- A run without a written question and prediction does not start. Inspect each result before scheduling the next.
- Never rerun a measurement whose commit, configuration and host have not changed. Find the baseline in LOG.tsv and reuse it.
- Repeats exist only at rung 5, or when a result lands inside the predeclared noise band around its decision threshold.
- Wrap each recurring procedure (build, run, parse) in one script that writes its full output to a log file, appends the LOG.tsv line itself and prints a single summary line. Results enter the log from the script, not from memory.
- Wait on a long job with one blocking command. Do not poll.
- Use package or crate scoped builds and focused checks. Reuse build caches and compatible artifacts.
- No builds and no competing CPU or I/O on the host during a performance measurement.
- Add a test only where a change puts at risk a correctness invariant that no existing test or oracle covers. No coverage backfill, no scaffolding, no test per function.
- During iteration run the narrowest check that exercises the change. Run the repository's required final gates once per integrated candidate, and again only after a relevant change, a failure or a demonstrated coverage gap.
- Never weaken a necessary correctness, security, concurrency, portability or integrity check to save time or tokens.

Paid API and cloud resources are for authorised, frozen, full live benchmark runs only. No paid tuning, subsets, smoke tests, diagnostics or preparation. Develop on owned, local or free resources. Respect existing authorisations and budgets and do not invent a new allowance: when more is needed, the reviewable run manifest goes to PENDING.md. Every cloud run has a deadline, a result-recovery step and verified resource deletion.

Paid runs are ordered cheapest first: a candidate that fails a cheaper sibling row is not run on a dearer one. A paid run stops the moment its target becomes unreachable (for a perfect-score row, the first verified wrong answer), and the partial result is logged as development evidence. Use batch pricing and prompt caching wherever latency is not the measured quantity. Cache intermediate artifacts (ingestion, extraction, embeddings) keyed by source hash, model and prompt, so an unchanged stage is never paid for twice. Never cache or replay final answers. Each manifest states its cost ceiling and which stages every repeat re-runs.

TOKEN DISCIPLINE

Every token must buy information that changes a decision.
- Read once. Search for symbols and line ranges, open ranges instead of whole files, and record what you learned in FILEMAP.tsv so a file is reopened only after it changes.
- Batch independent reads and searches into one step.
- Keep working context below the model's long-context pricing threshold where it has one. Compact right after a NOW.md update, never in the middle of an experiment.
- Send all build, test and benchmark output to a log file on disk. Bring back only the parsed metrics or the failing lines. Never print a full log, diff or file into context.
- Refer to artifacts by path and commit, and to rows, hypotheses and experiments by ID. Do not paste or re-describe them.
- Do not reread a file after patching it or re-check a result a tool already returned.
- No narration, progress reports, restated instructions, summaries of finished steps or plans offered for approval. The main thread emits one sentence, and only for a finding, a decision or a pending authorisation.
- No inline code comments except an essential non-obvious invariant or contract.
- Edit with minimal patches. Do not rewrite a file to change a few lines.
- Research each topic in one pass from primary sources and leave one research line in LOG.tsv so it is never repeated.
- Drop a hypothesis the moment its rejection threshold is hit. A rescue attempt needs a new card.
- Thrift removes waste only. It never trims correctness, evidence or the completion standard.

ANTI-OVERFITTING

Non-negotiable:
- All previously inspected benchmark queries and results are exposed development evidence. Reshuffling them does not restore an untouched holdout.
- Separate development, selection and fresh confirmation. Log every look and every configuration-selection decision.
- Freeze source, configuration, metric definitions, budgets and acceptance criteria before confirmation.
- If a confirmation influences another change, retire it as development evidence.
- Validate transfer across independent datasets, dimensions, metrics, distributions and update patterns.
- Use independent exact or high-precision oracles for correctness where appropriate.
- Retain complete recall/throughput curves, tail latency, memory, build and update costs, and regressions.
- Compare competitors under matched conditions.
- For QA, separate retrieval improvement from changes in reader, judge, prompt, context budget and inference cost.
- No query recognition, cached benchmark answers, label leakage or hidden benchmark-specific exceptions.
- Close residual errors by error class, never by item. A fix aimed at an identified benchmark question or query is a counterfeit exit. Each fix names the class it addresses and is validated on items it was not developed on.
- A mechanism that adapts to observed queries is evaluated only on queries it has never seen.
- Legitimate workload specialisation must be explicit, frozen and tested across multiple datasets of that workload.

INTEGRATION

Commit with the repository's required signing, DCO and message conventions. Create or update reviewable PRs. No stale-base merges. Do not merge or release without authorisation: those requests go to PENDING.md.

BOOTSTRAP

First launch only. Skip this section when NOW.md exists.

Start in /Volumes/A/holographic-memory. Read once, before changing anything:
- The repository's current agent instructions.
- /Volumes/A/.hms-lb/vdbbench/STATE.md
- /Volumes/A/.hms-lb/STATE.md
- The result tables of the benchmark reports and committed artifacts those records point to, not their raw logs.

Revalidate rather than assume:
- Prior work is in /Volumes/A/.hms-wt-vdbbench-perf, branch perf/vdbbench-low-level, last recorded head 74bb9fa8.
- PRs: https://github.com/writerslogic/warm/pull/425 and https://github.com/zilliztech/VectorDBBench/pull/898.
- The previous Azure resource group was deleted. Completed Azure ANN results are archived. The Azure VectorDBBench workload never started, so no Azure VDBBench score exists. Preparation artifacts are retained under /Volumes/A/.hms-lb/vdbbench/pr898-perf/cloud-artifacts. Old launch scripts contain expired deadlines and references to deleted resources.

Preserve unrelated user changes. Do not reconstruct completed investigations: consult their evidence and identify what remains unresolved.

Then:
1. Create the mission directory files. Write MATRIX.tsv, the ownership column of FILEMAP.tsv and SPECIALIST.md.
2. Audit each accuracy row's instrument (gold labels, judge) before any mechanism work on it.
3. Write a row's primitive-event accounting, ceiling and assumption list when that row is first worked, not for every row up front.
4. If SEEDS.md exists, assign each seed to a lane. The lane's specialist cards a seed, with its prior-art check, before it can be ranked.
5. Launch a specialist for each lane that has a bounded first question.
6. Write NOW.md, schedule the first decisive experiments and enter the operating loop.