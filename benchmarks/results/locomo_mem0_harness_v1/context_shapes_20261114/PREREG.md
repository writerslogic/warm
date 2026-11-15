# Pre-registration LOCOMO-3 (2026-11-14): context shape, written before any candidate metric or paid call

Harness (Mem0 4b61c5d), gpt-5-2025-08-07 answerer and judge, top-k 200 cutoff, 4,096 completion tokens, 1,540 questions: unchanged.
Facts read from the harness before design: format_search_results sorts items by score and slices [:200]; the answer prompt then
re-sorts the (at most 200) items by created_at, stably, and prints "(weekday, date) memory". All turns of a session share one
created_at, so inside a session the reader currently sees turns in descending retrieval score, not in dialogue order. Item text
has no length limit. Only the item count (200) is bounded.

Budget: OpenAI hard cap 48 USD for the whole task (ledger: spend.jsonl, summed from the usage logs of every paid process);
dev pilot at most 12 USD.

Splits (fixed now by conversation id): development = conversations 0-4 (762 questions with category 1-4 and listed evidence or not
counted separately), held-out = conversations 5-9. Offline selection of the merge parameters uses development only.

Candidates (adapter-side, WARM engine unchanged; env WARM_CONTEXT selects):
 O  "ordered": the same top-200 turns as baseline, one item per turn; created_at = session epoch + 1 s per turn index, so the
    reader sees each session in dialogue order. Retrieval set identical to baseline.
 M  "merged": recall k=400; walk hits in rank order, keep a selected set of turns; stop before the item count (maximal runs of
    consecutive turns of one session, with a one-turn gap g between selected turns filled when g=1) exceeds 200 or the turn count
    exceeds T. Each run is one item: its turns joined by newline in dialogue order, created_at = session epoch + first turn index.
    Item count <= 200 always. Grid fixed now: g in {0,1}, T in {250, 300, 400}.
 Not tried (recorded as untested): image-tag compaction (changes evidence text; no hypothesis it helps), near-duplicate removal
 (measured for frequency only), cross-session diversification.

Offline metrics (zero spend, cached query embeddings, private store copy, server --max-top-k 400): all-gold-in-context rate per
category (gold = evidence ids split on ; and ,), evidence recall, item count, turn count, mean prompt tokens (o200k) of the real
harness prompt. Baseline = fresh recall k=200 with the same cached embeddings.
M* selection from the six grid points on development: highest multi-hop all-gold; subject to no category all-gold point estimate
below baseline and mean prompt tokens <= 1.35 x baseline mean; tie to fewer prompt tokens.

Paid development pilot (development conversations only, at most 2 candidates: O and M*): every baseline-wrong development
question (from per_question.jsonl) plus, per category, a seed-0 random sample of baseline-correct development questions of size
max(failures in that category, 8) (capped at the category's count). Same answerer, judge, prompt, settings. Cache reuse is allowed only for byte-identical prompts.
Selection rule: estimated development accuracy = sum over categories of [F_c * fix_rate_c + S_c * keep_rate_c] / N_dev, where F_c and
S_c are the baseline development failures and successes in the category and rates are the pilot rates. Highest estimate wins; ties to
fewer mean prompt tokens. The estimate is biased up for any candidate (failures can flip on a rerun: 9 of 22 empties did), which
does not affect the choice between the two.

Confirmation: one fresh full run of 1,540 questions with the chosen configuration, ingest-count gate before any answer call, no
checkpoint reuse, call cache not consulted for the confirmation (a fresh cache file). Conversation order 5-9 then 0-4. If the cap
stops it, only whole completed conversations are reported, labelled partial, with old WARM and Mem0 on the same conversations.
Report: paired bootstrap (10,000 resamples, seed 0) vs Mem0's file, overall and per category, empty-answer count; a category still behind is reported as behind.

## Result of the offline grid and the registered M* (offline.txt, development conversations 0-4, nothing paid yet)
Baseline mean prompt 10,455 tokens (threshold 1.35x = 14,114). Grid points with no category all-gold below baseline and prompt <= threshold:
M_g0_T250 (12,224), M_g0_T300 (13,944), M_g1_T250, M_g1_T300 (13,395). Highest dev multi-hop all-gold among them: M_g0_T300 (.908 vs baseline .838). M* = merged, gap 0, 300 turns, recall k=400.
Both T400 points and nothing above 14,114 tokens were excluded by the token rule. O uses the harness's human-readable date form (the fraction in created_at makes the harness parse it);
the baseline prompt prints the ISO date, so O also changes the date format; this is part of O.
Pilot: 130 questions (64 baseline failures + 66 sampled successes; pilot_questions.json), conversations 0-4, candidates O and M*.

## Amendments before any paid call
- offline.txt base row recomputed with the adapter's exact baseline created_at (ISO date, no fraction); PREREG token figures (10,455 baseline) are those.
- Gate additions: the live client's turn order and epochs (WARM_INDEX_DIR) must equal the dataset's turn table by content, and no two sessions of a conversation may share a date_time.
- Pilot: O then M*, run one after the other, caps 4.5 and 6.5 USD (sum 11), distinct phases pilotO and pilotM; M* with --max-top-k 400. Report empties and length-finishes per candidate.
- Confirmation structure: two separate drive.py invocations. First conversations 5-9 (own ingest and gate), cap so that the task total stays below 47 USD. Conversations 0-4 are launched
  only if (spent so far + measured cost per question of the first batch x 762 + 1 USD) <= 47 USD; otherwise the held-out half is the confirmation and is reported as such. Projection uses the
  measured per-question cost of the chosen candidate, not the earlier run's. A cap hit inside a batch invalidates that batch's partial conversations: only complete conversations are reported.

## Pilot outcome and chosen configuration (written before the confirmation run)
pilot_analysis.txt: estimated dev accuracy O 0.9449, M* 0.9407 (baseline dev 0.9160). By the selection rule O is chosen (higher estimate; fewer prompt tokens too: 10,989 vs 13,939 mean). Empties among pilot answers:
O 7, M* 8 (length finishes of 130 answer calls 7 and 8): merging did not reduce the reasoning exhaustion. Pilot spend O 4.339, M* 4.772 USD (sum 9.111 of 12).
Chosen: WARM_CONTEXT=ordered, server --max-top-k 200, otherwise the earlier configuration.
Confirmation budget check (rule above): measured pilot cost O 0.0334 per question (a sample enriched in failing, reasoning-heavy questions); the earlier full run averaged 0.027. Conversations 5-9 (778 questions) are launched first.
Launch note: the first confA launch failed in ingest-only, before any answer call (zero LLM spend), on an OpenAI embeddings HTTP 500 the adapter did not retry; 500 added to the retried statuses (429, 500, 502, 503, 504) and confA relaunched from fresh directories.

## Confirmation outcome and the budget rule applied
Batch 1 (conversations 5-9, 778 questions, WARM_CONTEXT=ordered) cost 21.879 USD, 0.0281 per question (confA-usage.jsonl). Task LLM spend after it: 30.991 USD
(pilotO 4.339, pilotM 4.772, confA 21.879; embedding calls are not metered by the wrapper and are estimated under 0.05 USD in total). The rule for batch 2 (conversations 0-4,
762 questions): 30.991 + 762 x 0.0281 + 1 = 53.4 USD > 47, so it was not launched. The held-out half is the confirmation and is reported as such; conversations 0-4 were only used in the pilot sample.
