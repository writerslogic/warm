#!/bin/bash
cd ~/w/kill/round2
export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
O=~/w/out2
for src in locomo lme; do for s in 0 1; do
  (~/v/bin/python k5b.py tune --source $src --seeds $s --out $O/b_tune_${src}_s$s.json > $O/b_tune_${src}_s$s.log 2>&1 &)
done; done
