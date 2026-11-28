# Held-out query sets for the search-benchmark-game work

Every speed-up must hold on queries nobody tuned on, and must be exact on queries beyond the benchmark's
962. These sets are the first check; `../sbg_fuzz` is the second. The rules below were written before any
set was generated. A rule that changes is recorded in "Changes" first, with the reason.

## Rules (from the benchmark's own scripts and queries-full.txt)

Source: `extract_queries.py` and `format_queries.py` of the benchmark fork, and the shapes actually present in
`queries-full.txt` (962 queries) and `queries-tpuf.txt` (31).

Source query to benchmark queries:

1. The source query is used as written. It must match `^[a-z ]+$` (lowercase letters and spaces); a source that
   does not match is dropped, never rewritten (`LETTERS_ONLY` in `format_queries.py`).
2. It is split on `\s+` into words. A source needs at least 2 words.
3. The phrase `"w1 w2 ... wn"` must have at least 1 hit in the benchmark index (5,032,104 English Wikipedia
   documents, unstemmed, one chunk per article, positions kept), counted by the engine's served COUNT. This is the
   benchmark's "at least two terms and at least 1 hit as a phrase" rule. A source without a hit is dropped.
4. Every surviving source yields, in this order, with the tag layout of `queries-full.txt`
   (`[shape, "global" (not on phrase or term), "<shape>:num_tokens_N"]`):
   - `intersection`: `+w1 +w2 ... +wn`
   - `phrase`: `"w1 w2 ... wn"`
   - `union`: `w1 w2 ... wn`
5. `term`: the benchmark has one (`the`), plus two in `queries-tpuf.txt`, added by hand "with different term
   frequencies". Rule here: one word of each surviving source, chosen by the seeded generator. Tag `["term"]`.
6. `intersection_union` and `negated` are hand-written in `queries-full.txt` (40 and 19 queries), not derived by
   any script. Extension, with a deterministic rule read off those queries and recorded before use:
   - `intersection_union`: the first 4 words of the source; exactly one is required (`+w`), the rest optional. The
     required position is 0 or 1, alternating by source index (`+climate policy`, `remote +work`).
   - `negated`: the first 3 words; the first is required, the next 1 or 2 (alternating by source index) are negated
     (`+python -snake -monty`, `+python -snake`). A source whose required word repeats in the negated words, or
     whose negated words repeat, yields no negated query.
7. `two-phase-critic` (`+"the who" +uk`) and queries that exist once for a specific purpose are not generated.

Recorded discrepancies. `format_queries.py` is Python 2, tags only a random 10% of phrases as `phrase`, and writes
`num_token_N` tags; the shipped `queries-full.txt` instead tags every phrase and uses `<shape>:num_tokens_N`. The
shipped file is the reference: its triples (299 intersection, 300 phrase, 299 union) are replicated, the
script's 10% sampling is not. The AOL log that the benchmark's queries came from is not used here (leaked query
log).

Sets are written in the `queries-full.txt` JSONL format. The category of a query is its first tag.

## Sources

`$SBG_HOLDOUT_PRIVATE` is a private directory outside the repository. It holds `raw/` (downloads), `sealed/` (sealed sets, mode 400) and the
vocabulary file; the generator takes every path as an argument.

- NQ-open (Natural Questions open, CC BY-SA 3.0), the official release in
  `google-research-datasets/natural-questions`, `nq_open/`. Downloaded into
  `$SBG_HOLDOUT_PRIVATE/raw/` (untrusted; parsed with `python3 -I` from outside that directory).

  | file | URL | SHA-256 | lines |
  |---|---|---|---|
  | `NQ-open.train.jsonl` | https://raw.githubusercontent.com/google-research-datasets/natural-questions/master/nq_open/NQ-open.train.jsonl | `0c93aaf13b12f855628489665e4ef85ffb8573ad444646f11f4fabdc57a0aabf` | 87,925 |
  | `NQ-open.dev.jsonl` | https://raw.githubusercontent.com/google-research-datasets/natural-questions/master/nq_open/NQ-open.dev.jsonl | `f15567f38099f3615f5b8a685c0aef449c11ad90d3da3735e8d1b98115b40616` | 3,610 |

- df-stratified random queries, from the vocabulary of the benchmark corpus alone (`sbg_fuzz` `vocab`: whitespace
  tokens and their document frequencies). Terms are drawn from log-decade df strata (1; 2-9; 10-99; ...; 10^6 and up)
  plus the 100 longest lists, a stratum chosen uniformly and then a term uniformly within it. Source length is 2
  words 60%, 3 words 30%, 4 words 10%. The random sets have no source question: each category draws its own
  sources, and `phrase` takes real adjacent words (a random 2-4 word window of every 25th corpus document) so a
  phrase has at least one hit, as rule 3 requires.

## Splits

| split | open to development | sources |
|---|---|---|
| validation | yes | NQ train, seed 11 (yield sample) and the full train list; random seed A (101) |
| sealed test | no | NQ dev; random seed B (202) |

Sealed files are written to `$SBG_HOLDOUT_PRIVATE/sealed/` with mode 400. The repository holds the
generator, the seeds, the source hashes and the SHA-256 of every output file. It never holds sealed queries.
Sealed sets are never timed; they are reserved for the release decision. The random sets are sealed by
convention only: the generator, seed and corpus are public, so anyone can regenerate seed B. Do not.
Deduplication: the sealed sets drop any query that is in validation, `queries-full.txt` or `queries-tpuf.txt`;
random B drops anything in random A.

## Long-query holdout (natural-language unions of 20 to 60 terms)

The tpuf gap is mostly 44- and 57-term questions, so the next engine work needs long unions held out too.
Written before generation. Source: Project Gutenberg public-domain English books (not Wikipedia), fetched from
the `gutenberg.pglaf.org` mirror (`/cache/epub/<id>/pg<id>.txt`); SHA-256 of every file in `sources.json`. Text
is public domain in the US; the Project Gutenberg license text is stripped with the header and footer.

Sampling rule:

1. Take the text between the `*** START OF ... EBOOK` and `*** END OF ... EBOOK` markers; join whitespace.
2. Split into sentences after `.`, `!` or `?` followed by whitespace.
3. Apply the benchmark's corpus transform (`corpus_transform.py`: lowercase, every run of non-letters becomes one
   space) and split on whitespace. This is the corpus's own tokenization, and replaces the `LETTERS_ONLY` drop
   (almost no natural sentence of 20+ words is free of punctuation, so dropping would leave nothing).
4. Keep sentences with 20 to 60 words inclusive. Drop repeats.
5. Shuffle the eligible sentences with the seeded generator (Fisher-Yates over splitmix64) and take the first 400.
6. The query is the union: the words joined by single spaces, repeats kept as written (as a natural question
   would be). Tags `["union", "global", "union:num_tokens_N"]`. No phrase-hit rule: a union has a hit when any
   word is in the index.

Splits by book, disjoint. Validation (open), ids 1342, 2701, 84, 11, 1661, 98, 2600, 345, 174, 76, seed 301.
Sealed test, ids 1400, 1260, 768, 161, 158, 1184, 2554, 205, 2814, 3207, seed 302; it also drops any query in
validation.

## Query-length sweep (open, never sealed)

Unions of 1, 2, 4, 8, 16, 32, 48, 63, 64, 65, 96 and 128 distinct terms, 60 per length (seed 401), terms drawn
by df stratum as in the random sets (stratum uniform, then term uniform, terms distinct within a query, order
shuffled). It exists to find the cliff at `MAX_TERMS = 64` in `src/core/documents/impact.rs` (above it ranking
falls back to exhaustive), so it is open. Time it per length, not as one pooled set.

## Yield and primary holdout

Phrase-hit counts are the served COUNT of the quoted source on the 5M-document benchmark index (an older-format
index of the same corpus, opened read-only; its files were unchanged by the run).

| source | questions | letters-only, 2+ words | phrase hit >= 1 |
|---|---|---|---|
| NQ train, seeded sample of 1,000 (seed 11) | 1,000 | 762 | 1 |
| NQ train, all | 87,925 | 68,077 | 122 (0.18%) |
| NQ dev, all | 3,610 | 2,848 | 3 |

Natural questions are almost never verbatim Wikipedia text, so the phrase-hit rule leaves 122 train and 3 dev
sources. NQ cannot give 300 per category, so by the rule above **the df-stratified random set is the primary
holdout and NQ is secondary**. The rule was not loosened (no n-grams of questions, no dropped hit requirement) to
raise the NQ yield.

Counts per category after deduplication (`outputs.json` has hashes and seeds):

| set | split | term | intersection | phrase | union | intersection_union | negated |
|---|---|---|---|---|---|---|---|
| random_A | validation | 391 | 400 | 392 | 400 | 400 | 400 |
| random_B | sealed | 371 | 400 | 384 | 400 | 400 | 400 |
| nq_train_validation | validation | 81 | 122 | 122 | 122 | 115 | 98 |
| nq_dev_sealed | sealed | 1 | 3 | 3 | 3 | 3 | 3 |
| long_validation | validation | | | | 400 | | |
| long_sealed | sealed | | | | 400 | | |
| length_sweep | open | | | | 720 | | |

Short categories are the term and phrase dedup against the benchmark's own queries and within the set. The corpus
used for vocabulary and phrase windows is the 5,032,104-article `corpus.json` (SHA-256 in `outputs.json`); its
vocabulary file has SHA-256 `fa4b997e2bfb498d72a60bc7d7d81058738ee90a51841b9faf65fda00045a073`.

## Changes

None before generation. The long-query and length-sweep sections were added after the first draft and before their sets were generated.
