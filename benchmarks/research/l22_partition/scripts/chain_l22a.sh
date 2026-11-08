#!/bin/bash
# L22 kill test A chain on wl-03: three configs, sequential, timing lock held per arm.
cd ~/bigann/piso/l22a || exit 1
mkdir -p out
PY=~/bigann/s0/venv/bin/python
T0=$(date +%s)
run() { # tag args...
  tag=$1; shift
  echo "== $tag start $(date -u +%H:%M:%S) load: $(cut -d' ' -f1-3 /proc/loadavg)" >> out/chain.log
  flock ~/hms-timing.lock env OMP_NUM_THREADS=8 OPENBLAS_NUM_THREADS=8 MKL_NUM_THREADS=8 taskset -c 0-7 nice -n 19 \
    /usr/bin/time -v -o out/time_${tag}.txt $PY -I scripts/l22a.py "$@" --out out > out/${tag}.log 2>&1
  echo "== $tag exit $? $(date -u +%H:%M:%S) elapsed $(( $(date +%s) - T0 )) s" >> out/chain.log
}
run 1M_sample --n 1000000 --train-sample 262144
run 3M_sample --n 3000000 --train-sample 262144
el=$(( $(date +%s) - T0 ))
if [ $el -lt 7200 ]; then run 3M_prefix1M --n 3000000 --train-sample 262144 --train-prefix 1000000; else echo "skipped 3M_prefix1M (elapsed $el s)" >> out/chain.log; fi
grep -h '^RESULT' out/*.log > out/RESULTS.txt
touch DONE
