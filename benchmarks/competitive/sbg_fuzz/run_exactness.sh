#!/bin/bash
# Exactness evidence for one build: qb7d on the 962 and 31 query sets, then the fuzz differential, per index.
# usage: run_exactness.sh BIN_DIR OUT_DIR QUERIES_FULL QUERIES_TPUF INDEX LABEL VOCAB CORPUS SLOTS FUZZ_QUERIES [THREADS]
# Each step writes OUT_DIR/<label>-<step>.{log,time}; a step is skipped when its .done marker exists. Finish markers,
# not process names, say when the run is over: OUT_DIR/<label>.done.
set -u
bin=$1 out=$2 full=$3 tpuf=$4 index=$5 label=$6 vocab=$7 corpus=$8 slots=$9 nfuzz=${10} threads=${11:-7}
mkdir -p "$out"
step() {
    name=$1
    shift
    [ -f "$out/$label-$name.done" ] && return
    start=$(date +%s)
    "$@" > "$out/$label-$name.log" 2>&1
    echo "exit=$? seconds=$(($(date +%s) - start))" > "$out/$label-$name.time"
    touch "$out/$label-$name.done"
}
step qb7d-962 "$bin/qb7d" "$index" "$full" "$threads"
step qb7d-31 "$bin/qb7d" "$index" "$tpuf" "$threads"
step fuzz "$bin/fuzz" "$index" "$vocab" --seed 1 --queries "$nfuzz" --threads "$threads" --out "$out/$label-fuzz" \
    --corpus "$corpus" --slots "$slots" --threshold DENSE_MIN_POSTINGS=512 --threshold CHAMPION_MIN_POSTINGS=32768 \
    --threshold CHAMPION_K=4096
touch "$out/$label.done"
