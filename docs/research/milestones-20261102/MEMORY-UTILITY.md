# Memory utility evidence and remaining gate

Milestone 4 remains open. Existing native contract tests and the completed installed
SDK tests cover temporal/source semantics and operational behavior. They do not
by themselves establish real answer utility or an improvement over a strong
reader baseline. No paid model or cloud work has been launched for this milestone
qualification.

## Preserve the completed development evidence

The repository already contains LongMemEval development experiments. The
[v3 summary](../../../benchmarks/results/longmemeval_dev_answers_v3_summary.json)
reports 100 development questions with the matched reader: 88 correct for its
operand selection, 83 for first-five selection, and 82 for knapsack selection.
The paired operand-minus-first-five difference is 5 percentage points, with
95% interval **−1 to +11 points**. The difference against knapsack is 6 points,
with interval +1 to +12 points. The stronger first-five result therefore prevents
using the weaker comparison as the required advantage. These are recorded
development results, not a new held-out evaluation or a freshly verified source-
matched result. The summary reports that held-out questions were untouched.

The [completed development manifest](../../../benchmarks/results/longmemeval_completed_dev_evidence_manifest.json)
covers four earlier development experiments. The
[production closeout](../../../benchmarks/results/longmemeval_production_closeout_v1.json)
explicitly records an interruption before reader generation or judging: three
completed retrieval packets and one interrupted ingest, with no QA result.
Neither that closeout nor the word "production" qualifies a completed live run.
Reusing these reports for context creates no authorization for additional paid
development calls.

The three inspected summary artifacts have SHA256:

- v3 summary: `c65e7f6d6637de680a4f13ed369ced17d04717f9a89d741ff375d4834610d03a`.
- completed-development manifest: `8c43ca1d376a3b939029436af7af9cd221763426a3b280f5f4665b9b1f497d4d`.
- production closeout: `11a4349d5b354abb0ad36bb50c39bac750eefcd27d18c9ab459d42bd456398d8`.

The full v3 report is not present at its original repository path; this inspection
hashes the existing summary and does not claim a new raw-response audit.

## Current qualification work

The existing frozen native checks passed 30 memory API, 15 conversation-memory
and 14 context tests. [PLATFORMS.md](PLATFORMS.md) records the completed live HTTP SDK
checks and acknowledged process-death recovery. These exercise explicit
supersession, valid-time and known-time separation, coexisting contradictory
claims, provenance, atomic retries, tenant separation and chained forgetting.
The synthetic examples are labeled as such.

[TEMPORAL-TRACE.md](TEMPORAL-TRACE.md) is research on sufficiency after withdrawals,
not a replacement for the utility gate. Its inputs are supplied observations and
programs, and its stronger symbolic control removes the proposed algorithm's
advantage. It supplies no extraction or real-reader result.

The remaining utility qualification must bind a real held-out workload to frozen
ingestion, evidence selection and reader behavior; audit current/historical facts,
contradictions, source attribution and forgetting; and retain every failed case.
Any answer-quality advantage must compare the strongest matched control at equal
model/context budgets, reach at least five percentage points and have a paired 95%
confidence interval excluding zero. Existing development outputs may guide the
protocol but cannot be relabeled held-out evidence. Local/free development remains
the rule; any paid run must be a user-authorized live benchmark.

## Existing free local model path

Inventory on 2026-11-03 found all three small pinned Qwen checkpoints under
`/Volumes/A/hms-models`: Qwen3-Embedding-0.6B, Qwen3-Reranker-0.6B and
Qwen3-4B-Instruct-2507. Their Hugging Face download metadata records the revisions
already pinned in `src/core/models/mod.rs`. Every required blob has now been
verified against its Git/LFS digest and frozen in `local-models-v1-freeze.json`.
The fresh release server/reader build and 32 model-unit tests pass; both existing
conversation-model cases are running, including the normally ignored actual
embedding test. This uses only local weights and free dependencies; it does not
establish HTTP lifecycle coverage, generated answers or a QA advantage.

The repository already has a truly local reader driver,
`benchmarks/public/longmemeval_weak_reader.py`, for the historical 100-question
development contract. Its frozen binary path is absent, so an unrelated fresh
binary must not be substituted into the old protocol. A new source-bound protocol
would be required before using the fresh build for development. The separately
named `longmemeval_local_reader.py` uses paid Modal execution and model judging;
it is not this local path and remains unlaunched. No new reader generations,
held-out QA or answer-quality results are recorded by this preparation.
