#!/bin/bash
# Phase 2: embedded-insert sweep, main vs new, 3 alternating runs, then phase 3 A/B in both orders.
source ~/.cargo/env
export JAVA_HOME=$HOME/jdk-21.0.8+9; export PATH=$PATH:$JAVA_HOME/bin
O=~/fair/out; L=$O/final2.log; : > $L
cd ~/fair/warm-main
grep -q write_path_scale_bench src/core/documents/search/tests.rs || cat ~/fair/scale-bench.rs.txt >> src/core/documents/search/tests.rs
for i in 1 2 3; do
  for t in main new; do
    [ $((i%2)) = 0 ] && t=$([ $t = main ] && echo new || echo main)
    (cd ~/fair/warm-$t; WARM_BENCH_DOCS=50000,150000,500000 CARGO_TARGET_DIR=~/fair/target-sweep-$t cargo test --release --lib write_path_scale_bench -- --ignored --nocapture 2>&1 | grep -E "^SCALE|^error|panicked" -A3 | sed "s/^/$t run$i /") >> $L
  done
done
echo "SWEEP DONE $(date -u +%T)" >> $L
cd ~/search-benchmark-game; export CORPUS=$PWD/corpus.json
run() { echo "=== $1 $(date -u +%T)" >> $L; rm -rf results results.json results-full.json
  make --no-print-directory bench-full ENGINES="$2" >> $L 2>&1; echo "exit $? $1" >> $L; cp results-full.json $O/$1.json; }
{ uname -a; lscpu | grep -E "Model name|^CPU\(s\)"; date -u; } >> $L
run final-A "warm-main-final warm-new-final warm-new-oldidx"
run final-B "warm-new-oldidx warm-new-final warm-main-final"
echo "FINAL2 DONE $(date -u +%T)" >> $L
