#!/usr/bin/env bash
# prof.sh <tag> <dataset> <build json> <ef> <rerank> <reps> <wheel>
cd ~/ab-a/harness
export LD_LIBRARY_PATH=$HOME/perf/ext3/usr/lib/x86_64-linux-gnu OMP_NUM_THREADS=1 RAYON_NUM_THREADS=1 WARM_BUILD_CACHE=$HOME/ab-a/cache WARM_BUILD_TAG=f21de886 PYTHONPATH=$HOME/ab-a/wheels/$7
PERF=$HOME/perf/ext3/usr/lib/linux-oem-6.8-tools-6.8.0-1005/perf
flock $HOME/hms-timing.lock taskset -c 4 $PERF record -e cpu-clock -F 4000 -o $HOME/ab-a/runs/perf-$1.data -- $HOME/ab-hms/venv/bin/python -u profq.py $2 "$3" $4 $5 $6 > $HOME/ab-a/runs/perf-$1.out 2>&1
$PERF report -i $HOME/ab-a/runs/perf-$1.data --no-children --sort dso,symbol --stdio 2>/dev/null | grep -v "^#" | head -45 | cut -c1-220 > $HOME/ab-a/runs/perf-$1.rep
echo PROF_DONE $1 >> $HOME/ab-a/runs/perf-$1.out
