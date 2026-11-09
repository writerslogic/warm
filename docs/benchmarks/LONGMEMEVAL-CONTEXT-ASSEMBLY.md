# LongMemEval_S: what the reader sees (dev100 screen)

Status: partial. Dev-only screen; no 500-question confirmation was run. Tags: [M] measured, [I] inferred.

## Fixed
Reader gpt-4o-2024-08-06 (temperature 0), the official reading TEMPLATE unchanged, judges = official `evaluate_qa.py` (gpt-4o and gpt-4o-mini). Only the history content changes. The Engram board fixes only the judge; reader and prompt are free, we kept them fixed anyway. Dev = frozen dev100 (LongMemEval_S has exactly 500 questions, so no disjoint dev set exists outside the 400 held-out; dev100 is the only split not used for the held-out claim, and the 400 were already exposed to the earlier k30+preopen choice).

## Error breakdown [M] (`benchmarks/public/lme_context/breakdown.py`, zero API)
- gpt-5-mini k30+preopen: 34-37 wrong per pass; 51 wrong in at least one of 3 passes; 21 wrong in all 3 (multi-session 10+2 abstention, knowledge-update 4+1, temporal 3, single-session-user 1). Of those 21, the answer evidence is fully inside the context for 15, partial for 3, absent for 1, 2 are abstention questions: the persistent loss is aggregation/reading, not retrieval.
- gpt-4o k10: 75 wrong of 500 (multi-session 33, temporal 19, knowledge-update 10, preference 7, assistant 4, user 2); evidence fully in context for 42, partial 24, none 6, abstention 3.
- gpt-4o k10 on dev100: 11 wrong (multi-session 5, preference 2, one each user, update, assistant, temporal).

## Dev100 screen [M] (k=10 base; score = correct of 100, gpt-4o judge / gpt-4o-mini judge)
| arm | what the reader additionally sees | score |
|---|---|---|
| control (fresh rerun; frozen run was 89/88) | nothing | 90 / 88 |
| T | resolved dates for relative expressions in the question, per-session "N days before" | 90 / 90 |
| F | up to 80 dated facts from the 8 top sessions | 87 / 87 |
| S (two samples) | 2-sentence summaries of the 15 top sessions | 93 / 93 and 89 / 91 |
| Q | question-text routing to T/F/S per question shape | 90 / 89 |
| ST | S + T | 92 / 94 |
| S30 | summaries of the 30 top sessions | 92 / 93 |

Repeat noise [M]: the same S arm scored 93 then 89 under the gpt-4o judge, so single-pass differences below ~4 questions are noise. Reading [I]: flat extracted-fact lists hurt (temporal 21 vs 24); summaries, alone or with date references, are +1 to +4 over control and are not established above noise; T and routing are neutral. For reference the gpt-4o gold-session oracle scores 91 on dev100, so ST/S30 sit at, not above, that ceiling within noise.

## Not done (funds)
The OpenAI key reports "Billing hard limit has been reached" (no Batch API) and the OpenRouter balance fell to about $8, below one 500-question gpt-4o pass (~$11). Not run: the 500 confirmation with repeats, ablations, gpt-5-mini cross-reader, the factorial, update chains through the memory API (extraction slots are free-form: 47k distinct slot strings in 3,600 sessions, so exact-slot chains would rarely fire), and null-calibrated abstention.

## Cost [M]
Extraction on Modal H100: $8.29 for 4,775 dev sessions (the full 19,195-session haystack would be ~$33). Reader+judge via OpenRouter: see `openrouter_usd` in the results JSON.
