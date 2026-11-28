#!/bin/bash
source ~/.cargo/env
O=~/fair/out; L=$O/final3.log; : > $L
cd ~/fair; rm -rf warm-new2; mkdir warm-new2; tar xzf warm-new2.tgz -C warm-new2
for i in 1 2 3; do
  for t in main new2; do
    [ $((i%2)) = 0 ] && t=$([ $t = main ] && echo new2 || echo main)
    (cd ~/fair/warm-$t; WARM_BENCH_DOCS=50000,150000,500000 CARGO_TARGET_DIR=~/fair/target-sweep-$t cargo test --release --lib write_path_scale_bench -- --ignored --nocapture 2>&1 | grep -E "^SCALE|^error|panicked" -A3 | sed "s/^/$t run$i /") >> $L
  done
done
echo "SWEEP2 DONE $(date -u +%T)" >> $L
