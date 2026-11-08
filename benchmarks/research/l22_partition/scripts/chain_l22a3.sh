#!/bin/bash
cd ~/bigann/piso/l22a || exit 1
mkdir -p out3
echo "== a3 start $(date -u +%H:%M:%S) load: $(cut -d' ' -f1-3 /proc/loadavg)" >> out3/chain.log
flock ~/hms-timing.lock env OMP_NUM_THREADS=8 OPENBLAS_NUM_THREADS=8 taskset -c 0-7 nice -n 19 \
  /usr/bin/time -v -o out3/time.txt ~/bigann/s0/venv/bin/python -I scripts/l22a3.py --out out3 > out3/a3.log 2>&1
echo "== a3 exit $? $(date -u +%H:%M:%S)" >> out3/chain.log
grep -h '^RESULT' out3/a3.log > out3/RESULTS.txt
touch DONE3
