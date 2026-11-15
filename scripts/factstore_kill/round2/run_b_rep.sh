#!/bin/bash
# usage: run_b_rep.sh SRC "seeds-csv" ...  (one worker per argument)
cd ~/w/kill/round2; export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
src=$1; shift
for grp in "$@"; do
  (setsid nohup ~/v/bin/python k5b.py report --source $src --seeds $grp --wanted ~/w/out2/wanted_$src.json --out ~/w/out2/b_rep_${src}_s${grp//,/_}.json > ~/w/out2/b_rep_${src}_s${grp//,/_}.log 2>&1 < /dev/null &)
done
