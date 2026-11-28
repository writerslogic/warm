# Carried-state probe for the lexical search-benchmark-game engines

Status: run on `warm-sbg-c3` (GCP c3-highcpu-8), 2026-11-28 03:20 to 03:30 UTC, against the game checkout's own builds and indexes (untouched), `taskset -c 2`, JDK 21.0.8. Raw: `probe.json` (per-repeat medians), `probe.txt`.

Objection: an engine that keeps work between queries (result or postings caches, warmed per-query state) looks faster in a benchmark that repeats queries than one that does not. Which of infino 0.11.26, infino main, Lucene 10.5.0 and Lucene 10.5.0-bp do this?

`probe.py` drives each engine's own `make -s serve` process over the benchmark protocol and times every round trip. Per engine and command (default `COUNT`, `TOP_10`), in fresh processes, after a warmup of 200 distinct queries that appear in no measured set:

- `hot`: a fixed set of 3 queries repeated 500 times (`+william +wallace`, `montgomery college`, `john klein`).
- `mixed`: 500 distinct queries from the 962 in `queries-full.txt`.
- `twice`: the same 500 distinct queries sent twice in one process. Pass 2 over pass 1 holds query cost fixed, so it measures what the engine remembers, not what the queries cost.

`hot/mixed` is the comparison the objection asked for. It also reflects the cost mix of the 3 chosen queries, so the flag uses `pass2/pass1` (flag below 0.7, adjustable). Six repeats with alternating workload order; the figure is the median of per-repeat medians.

Run on the SBG VM once the coordinator releases it (engines built and indexed with `make compile index`):

    taskset -c 2 python3 probe.py --fork ~/search-benchmark-game --out results/

Validation: `python3 probe.py --selftest` checks that a memoising stub engine is flagged and a stateless one is not (pass).

Limits: the round trip includes pipe latency and, for Lucene, JVM scheduling; absolute microseconds are printed so that fast queries can be judged against that floor. Engines are probed as the SBG client would drive them, one query at a time, not under the harness's own warmup schedule.

## Result (median round trip, microseconds; 6 repeats)

```
engine                command       hot us  mixed us  pass1 us  pass2 us  hot/mixed  pass2/pass1  verdict
infino-0.11.26        COUNT          168.1     226.8     228.8     226.6       0.74         0.99  no carry detected
infino-0.11.26        TOP_10         126.2     250.5     246.7     230.4       0.50         0.93  no carry detected
infino-main           COUNT          221.0     262.2     260.1     260.2       0.84         1.00  no carry detected
infino-main           TOP_10         141.3     262.7     263.3     245.1       0.54         0.93  no carry detected
lucene-10.5.0         COUNT          646.0     974.5     946.1     669.7       0.66         0.71  no carry detected
lucene-10.5.0         TOP_10         589.0    1123.9    1111.5     806.7       0.52         0.73  no carry detected
lucene-10.5.0-bp      COUNT          451.7     770.3     754.4     531.3       0.59         0.70  no carry detected
lucene-10.5.0-bp      TOP_10         527.7     908.9     914.9     650.8       0.58         0.71  no carry detected
```

WARM rows are in `warm-0.7.4/` (engine directory `warm-074-old-bp` on the VM, the 0.7.4 build the fair probe used; run 03:32 to 03:34 UTC, lock held 03:32:08 to release below), same protocol:

```
engine                command       hot us  mixed us  pass1 us  pass2 us  hot/mixed  pass2/pass1  verdict
warm-074-old-bp       COUNT           36.9     192.8     193.7     176.6       0.19         0.91  no carry detected
warm-074-old-bp       TOP_10          49.2     176.2     177.0     158.9       0.28         0.90  no carry detected
```

Reading, as measured:

- infino 0.11.26 and infino main: pass2/pass1 is 0.93 to 1.00 in every repeat.
- Lucene 10.5.0 and 10.5.0-bp: pass2/pass1 is 0.67 to 0.76 across repeats, medians 0.70 to 0.73, for both commands. No cause is claimed.
- WARM 0.7.4: pass2/pass1 0.91 (COUNT) and 0.90 (TOP_10), hot/mixed 0.19 and 0.28. The expectation that 0.7.4 shows pass2/pass1 well below 1 was not met on the 500 distinct queries: a repeated set of 3 queries is 3.6 to 5.2 times cheaper than the mixed stream, but replaying 500 distinct queries gains only about 10 percent. Whatever 0.7.4 carries pays off on a small repeated set, not on a second pass over a wide one.
- hot/mixed is below 1 on every engine (0.19 to 0.84), so a benchmark that repeats a small query set understates per-query cost on all of them, most on WARM 0.7.4.
- Round trips include pipe latency, identical across engines.

WARM, #554 head `81315b0a` (engine `warm-new-final`, 5,032,104 docs; run 06:24 to 06:25 UTC, lock 06:24:15 to 06:25:27, fair agent paused meanwhile), rows in `warm-81315b0a/`:

```
engine                command       hot us  mixed us  pass1 us  pass2 us  hot/mixed  pass2/pass1  verdict
warm-new-final        COUNT          446.8     232.2     234.5     232.2       1.92         0.99  no carry detected
warm-new-final        TOP_10         229.5     191.5     193.1     191.5       1.20         0.99  no carry detected
```

- pass2/pass1 per repeat: COUNT 0.97 to 1.01, TOP_10 0.98 to 1.00. No carried state detected, matching the expectation of about 1.
- hot/mixed is 1.92 (COUNT) and 1.20 (TOP_10): the 3 hot queries are slower than the average mixed query on this build, where 0.7.4 made them 3.6 to 5.2 times faster. 0.7.4's advantage on a repeated small set is gone; the mixed-stream per-query time is 232 us (COUNT) and 192 us (TOP_10) against 193 and 176 for 0.7.4, so this build is slower on the mixed stream in this probe. Different index (the new build's own), so not a paired engine comparison.

The `fork` and `out` fields of the three `probe.json` files were redacted from absolute VM paths to relative names; no measured value changed.
