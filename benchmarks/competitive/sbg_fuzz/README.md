# sbg_fuzz: randomized exactness differential for the benchmark engine

Runs seeded random queries through the served path (`LexicalExecution::Auto`) and `LexicalExecution::Exhaustive`
and compares the count and the qb7d digest (FNV-1a over ordered ids, chunk indexes and score bits). `qb7d` itself
(the fixed-query-file differential) is built unchanged from
`../../results/search_benchmark_game_c3_v1/exhaustive_differential/qb7d.rs`.

Build from a checkout of the commit under test (the crate sits at `benchmarks/competitive/sbg_fuzz` inside it):

```sh
cargo build --release --bin fuzz --bin vocab --bin count_queries --bin qb7d
vocab corpus.jsonl > vocab.tsv        # document frequencies of the corpus the index was built from
fuzz INDEX vocab.tsv --seed 1 --queries 20000 --threads 8 --out OUT \
     --corpus corpus.jsonl --slots DOCS \
     --threshold DENSE_MIN_POSTINGS=512 --threshold CHAMPION_MIN_POSTINGS=32768 --threshold CHAMPION_K=4096
```

- Queries: terms by df stratum (1; 2-9; ...; 10^6+), half drawn from a window of a real document so intersections and
  phrases are not all empty; term, union, intersection, phrase, required plus optional, required plus negated; 1 to
  8 terms; k from {1, 2, 7, 9, 10, 11, 99, 100, 101, 999, 1000, 1001, 4095, 4096, 4097} or random up to 10,000;
  commands top-k, count, top-k plus count.
- Thresholds: every `--threshold NAME=VALUE` adds terms with df at VALUE-1, VALUE and VALUE+1 (or the nearest df
  present, reported in `OUT/boundary_terms.json`) alone and combined with a long list, over edge k values and
  all commands. `--slots N` adds `SLOT_SHARE = ceil(N / 32)`, the `postings * 32 >= slots` rule of
  `impact.rs::tabled`. A later change that adds a threshold adds a `--threshold` argument; nothing is hard-coded.
- Lengths 63, 64 and 65 terms (`--lengths`, `--length-reps`) in every operator shape, around `MAX_TERMS = 64`.
  The exhaustive side takes tens of seconds per such case at 5M documents, so each gets one k and command.
- `make_synth.py` writes a corpus whose terms have exact dfs at each threshold, for sizes where the real corpus has
  no term at the boundary. Build it with the same `build_index` and fuzz it with the same `--threshold` values.
- A mismatch (including an error on one side only) writes `OUT/repro-N.json`: query, k, command, and both result
  lists as id, chunk index, score bits. Exit status 2 on any mismatch.
- An index is opened by one process at a time; run the fuzz and qb7d runs of one index sequentially.
