# Comparator re-pin, local paired run (2026-11-03)

Machine: Apple M4, 10 logical cores, 32 GiB, macOS (Darwin 27.0.0), arm64; shared with other sessions
(load_1m at start of each run 4.6 to 5.8; the datasets ran concurrently with each other, so absolute
numbers are indicative; the new/previous ratios inside one run are the result). Harness commit 0ca75dc5
(main src plus the harness change). WARM = `--encode lexical`.

Pins, previous -> new (source):
- tantivy-py 0.26.2 -> 0.27.0 (bundles Rust tantivy 0.26.2 -> 0.27.0), https://pypi.org/project/tantivy/0.27.0/ (2026-10-22), crate https://crates.io/crates/tantivy (2026-10-17)
- Apache Lucene 9.12.3 -> 10.6.0, https://repo1.maven.org/maven2/org/apache/lucene/lucene-core/10.6.0/ (2026-10-29; sha256 pinned in warmbench/jars.py, sha1 matches Maven Central)
- SQLite 3.50.4 (CPython 3.13.9) -> 3.53.1 (CPython 3.13.15 standalone). sqlite.org lists 3.54.1 (2026-10-16); no CPython 3.13 build bundling it exists, so 3.53.1 is the newest reachable without changing the sqlite3 module.
- JVM: both Lucene versions ran on Homebrew OpenJDK 26.0.1 (the machine's JDK 17 cannot load Lucene 10, class version 65). The JVM is therefore a constant between the two Lucene rows.
  Modal image changed to openjdk-21-jdk-headless in modal_run.py; untested (needs Debian base with JDK 21).

Method: each comparator at its new and previous pin, plus WARM, in the same interleaved serve rounds
(5 rounds, order rotated; builds run per system, 2 repeats, not interleaved, so build_seconds ratios are the noisiest).
The replace/delete (durable ops) phase was skipped (WARM_BENCH_SKIP_OPS=1): per-operation fsync on a loaded disk took more than 10 minutes per dataset.
Rows: REPORT.md (full tables). Result files: light-text-<dataset>-local-macos-arm64.json.

## Rows that change class (WARM vs comparator), nfcorpus and memory
- build_seconds vs tantivy: TIE -> WIN on both datasets (tantivy 0.27.0 built slower than 0.26.2 here, 1.96x nfcorpus, 1.23x memory; unpaired, treat as unproven).
- cold_start_s vs tantivy (nfcorpus): LOSS 0.22x -> TIE 0.21x (interval width, not a speed change).
- No flips in qps, p50, p99, serving RSS, disk, nDCG@10, recall@100 on either dataset. Every query-latency WIN stays a WIN.
- Losses that persist against the new pins: build_peak_rss_mib vs SQLite FTS5 (LOSS), plus whatever REPORT.md shows as LOSS.

## Version effect (new / previous, qps higher is better)
- memory qps: SQLite 0.975, tantivy 1.035, Lucene 1.238. nfcorpus qps: SQLite 0.997, tantivy 0.939, Lucene 1.025.
- tantivy 0.27.0's block-max WAND does not show as a speedup through the tantivy-py single-query OR path used here (<= 3.5% either way, within noise).

## Stale gaps (benchmarks/competitive/GAPS.json, docs/benchmarks/*, README-block.md)
All 27 entries (Lucene 9.12.3: 5, SQLite FTS5: 13, tantivy: 9) were measured on `modal-x86_64-8vcpu-32g` against the previous pins and are stale until regenerated.
The two local flips above are in build_seconds/cold_start_s, not in a GAPS-listed query metric class.
Not hand-edited. `table` was not regenerated: it labels every Lucene row with the current registry display, so regenerating from the old Modal files would mislabel 9.12.3 data as 10.6.0.
tests/test_table.py::test_cited_files_are_committed_and_table_is_current fails on this branch for that reason until the Modal re-run below lands.

Modal re-run (regenerates the heavy result files; then run `./benchmarks/competitive/warm-bench table`):
`uv run --with modal modal run --detach benchmarks/competitive/modal_run.py --cap-usd 30 --shards 'text:beir:scifact,beir:nfcorpus,memory'`
