# Pre-registration LOCOMO-2 (2026-11-14), written before any candidate was measured

Free and offline: no provider key in any process environment (env -i HOME PATH only), query embeddings read from the existing
cache (memeval-20261112/locomo/cache.sqlite, text-embedding-3-small; a miss aborts), retrieval against a private copy of the
ingested store (~/pilot-out/store) served by the pinned pilot-memsrv with --max-top-k 200 (the flags of the paid run).

Observed before registering (descriptive, from the cause analysis, not from any candidate): cached query embeddings differ
from the live ones used in the paid run by about 1e-4 in score, so a fresh top-200 equals the stored one for only a third of
questions. The baseline for every comparison is therefore a fresh run of the unmodified server with the same cached
embeddings (base200.json), never the stored lists. Missing gold turns sit at ranks 201-400 (median 278) of a deeper list.

Metric per question (categories 1-4 with listed evidence; evidence ids split on ';' and ','): all-gold-in-200 (0/1) and
evidence recall. Splits: dev = conversations 0, 1, 2 (tuning only); held-out = conversations 3 to 9 (evaluated once, with the
single configuration chosen on dev). Intervals: paired bootstrap over questions, 10,000 resamples, seed 0, 95 percent
percentile, on candidate minus baseline.

Candidates (all fixed here; none added after seeing results). Context cap stays 200 items in every candidate.
 N(S): neighbour expansion. Take the top S hits (S in 10, 20, 50). For each, in rank order, add the previous and the next
       turn of the same session (ingestion order) if absent from the list. Added turns are inserted directly after their seed
       and the list is truncated to 200 from the tail.
 P(R): second hop, R in 20, 40. Query embedding e2 = unit(e_q + 0.5 * mean(embeddings of the top 3 hits)); a second recall at
       k=200 with the original query text; final list = top 200-R of hop 1, then the first R hop-2 items not already present,
       then the rest of hop 1 if slots remain.
 Not tried: per-speaker merge (this protocol already ranks one list per conversation), entity/date-seeded text rewriting and
 per-session diversification (no registered hypothesis from the cause analysis supports them; recorded as untested).

Selection on dev: among the five configurations, keep those with no category all-gold point estimate below zero on dev; pick
the one with the highest multi-hop all-gold difference; ties to the fewest components (N before P, smaller S or R).

Keep rule on held-out, for that single configuration: (1) multi-hop all-gold difference with the 95 percent interval above
zero; (2) no category's all-gold difference below zero on dev nor on held-out (point estimates); (3) retrieval latency, median
of per-question end-to-end recall time (all calls the method needs, two serial passes), at most 1.5x the baseline median;
(4) for any change to default engine behaviour: the repo's LoCoMo and LongMemEval retrieval gates do not regress, else it is
opt-in only. A configuration failing any condition is rejected.

## Amendments, still before any candidate was measured
Disclosure: phase0.py and diag.py (descriptive) read all ten conversations, including 3 to 9, before the candidates and their
parameters were fixed; the neighbour statistic (69 percent of missing multi-hop gold turns have an adjacent turn already in the
top 200) is pooled over all ten. Selection still uses dev only and the held-out evaluation is a single run of one configuration.
Added candidate, since recall's score normalisation depends on its candidate pool (the same query at k=200 and k=1000 gives
different top-200 lists):
 C(L): the same recall with candidate_limit L in 400, 1000 and k=200. No new embedding is needed.
Selection on dev now covers seven configurations (N10, N20, N50, P20, P40, C400, C1000). P is run only if the cache holds every
embedding it needs (turn embeddings of the "{speaker}: {text} [image tag]" content); otherwise it is reported as not measurable
offline. N needs turn order: the existing store has ingestTime per fact (one remember call per turn, in order), so adjacency is
ingestTime order within a session; if ingestTimes tie, N is recorded as not implementable on this store.
Changed question: the candidate's rendered top-200 set differs from the baseline's (base200.json, same cached embeddings). The
projection holds only if a paid rerun feeds the cached query embeddings; with live embeddings about two thirds of lists differ
from any earlier run anyway. Speaker merge does not apply (one tenant and one list per conversation).
Measured before running candidates: 1,226 of 5,882 ingested turn contents (the image-tagged ones) have no cached embedding and
the server does not expose stored embeddings, so P cannot be run on every question offline. P is therefore declared not
measurable and is dropped; selection covers N10, N20, N50, C400, C1000.
Stage A result (dev only, stageA.txt): N10 and N50 have a negative category all-gold point estimate on dev (multi-hop -.014 and
temporal -.011) and are excluded; of N20 (+.041), C400 (+.027) and C1000 (+.000) on multi-hop, N20 is selected and is the only
configuration run on the held-out conversations.
Stage B result (held-out 3-9, stageB.txt, single run): N20 multi-hop all-gold +.014 [-.024, +.053]. The interval includes zero
(condition 1 fails), and open-domain (-.027) and temporal (-.004) point estimates fall (condition 2 fails). N20 is rejected, no
configuration is kept, and nothing ships. Recall rows that printed nan came from questions with no resolvable evidence id (an
empty gold set); the rerun excludes those from recall differences only and leaves all-gold unchanged.
