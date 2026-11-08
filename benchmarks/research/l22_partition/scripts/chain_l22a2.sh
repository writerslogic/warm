#!/bin/bash
cd ~/bigann/piso/l22a || exit 1
mkdir -p out2
PY=~/bigann/s0/venv/bin/python
T0=$(date +%s)
run() { tag=$1; shift
  echo "== $tag start $(date -u +%H:%M:%S) load: $(cut -d' ' -f1-3 /proc/loadavg)" >> out2/chain.log
  flock ~/hms-timing.lock env OMP_NUM_THREADS=8 OPENBLAS_NUM_THREADS=8 MKL_NUM_THREADS=8 taskset -c 0-7 nice -n 19 \
    /usr/bin/time -v -o out2/time_${tag}.txt $PY -I scripts/l22a2.py "$@" --out out2 > out2/${tag}.log 2>&1
  echo "== $tag exit $? $(date -u +%H:%M:%S) elapsed $(( $(date +%s) - T0 )) s" >> out2/chain.log
}
run 1M --n 1000000
run 3M --n 3000000
grep -h '^RESULT' out2/*.log > out2/RESULTS.txt
touch DONE2
