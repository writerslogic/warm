#!/bin/bash
source ~/.cargo/env
O=~/fair/out; L=$O/final4.log; : > $L
cd ~/fair; rm -rf warm-new3; mkdir warm-new3; tar xzf warm-new3.tgz -C warm-new3
for i in 1 2 3; do
  for t in main new3; do
    [ $((i%2)) = 0 ] && t=$([ $t = main ] && echo new3 || echo main)
    (cd ~/fair/warm-$t; WARM_BENCH_DOCS=50000,150000,500000 CARGO_TARGET_DIR=~/fair/target-sweep-$t cargo test --release --lib write_path_scale_bench -- --ignored --nocapture 2>&1 | grep -E "^SCALE|^error|panicked" -A3 | sed "s/^/$t run$i /") >> $L
  done
done
echo "SWEEP3 DONE $(date -u +%T)" >> $L
