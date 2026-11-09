#!/usr/bin/env bash
# IL-S: elite candidates on warm-bench-icelake. Args: WARMSHA then run specs "label:nprobe:reorder" ... Wheel from perf/l22-apex.
set -uo pipefail
WARM=$1; shift; SPECS=("$@")
export PATH=$HOME/.cargo/bin:$HOME/.local/bin:$PATH
C=$HOME/il; LOG=$C/logs; ST=$LOG/status-g2.txt; IN=$C/in; W=$C/bab; PY=$C/venv/bin/python; O=$C/out-g2
HARNESS=56fc6e6b4ee74f5ae4601982b6cafab0ee8899eb
D=$W/data/MSTuring-30M-clustered; G=$D/29998994/final_runbook.yaml
CFG=neurips23/streaming/warm/config.yaml
st(){ echo "$(date -u +%FT%TZ) $*" >> "$ST"; }
abort(){ st "ABORT $*"; echo "$*" > "$C/ABORT-G2"; ( cd "$W" && git checkout -- "$CFG" ); exit 1; }
rm -f "$C/ABORT-G2" "$C/DONE-G2" "$C/SKIP-G2"; mkdir -p "$O"
st "chain-e start pid=$$"
[ "$(git -C "$W" rev-parse HEAD)" = "$HARNESS" ] || abort "harness HEAD"
[ -z "$(git -C "$W" status --short)" ] || abort "harness dirty: $(git -C "$W" status --short | head -3)"
rustc +1.99.0 -V >> "$ST" 2>&1 || abort "rust 1.99.0"
git -C "$C/src" checkout -q -- . >> "$LOG/git.log" 2>&1
git -C "$C/src" fetch -q origin perf/l22-apex >> "$LOG/git.log" 2>&1 || abort "fetch warm"
git -C "$C/src" checkout -q $WARM >> "$LOG/git.log" 2>&1 || abort "checkout warm"
WARMFULL=$(git -C "$C/src" rev-parse HEAD); [ "$WARMFULL" = "$WARM" ] || abort "warm HEAD $WARMFULL"
git -C "$C/src" status --short | head -3 > "$O/warm-status.txt"; [ ! -s "$O/warm-status.txt" ] || abort "warm dirty"
st "warm at $WARMFULL; harness ok"
[ "$(stat -c %s "$D/30M-clustered64.fbin")" = 11999597608 ] || abort "fbin size"
[ "$(ls "$G" | wc -l)" = 640 ] || abort "gt count"
bad=0; for f in $(cat "$C/gtlist.txt"); do [ "$(stat -c %s "$G/$f" 2>/dev/null)" = 8000008 ] || bad=$((bad+1)); done
[ $bad = 0 ] || abort "gt-size bad=$bad"
( cd "$W" && "$PY" create_dataset.py --dataset msturing-30M-clustered ) > "$LOG/create_dataset-g2.log" 2>&1 || abort "create_dataset"
st "data ok"
[ -d "$C/wheels-s2e" ] || mv "$C/wheels" "$C/wheels-s2e"
mkdir -p "$C/wheels"; rm -f "$C"/wheels/*.whl
st "wheel build start"
( cd "$C/src" && . "$C/venv/bin/activate" && RUSTUP_TOOLCHAIN=1.99.0 CARGO_TARGET_DIR="$C/target" CARGO_BUILD_JOBS=8 \
  maturin build --release --features python,experimental -i "$PY" -o "$C/wheels" ) > "$C/build-g2.log" 2>&1 || abort "wheel build"
[ "$(ls "$C"/wheels/*.whl | wc -l)" = 1 ] || abort "wheel count"
WHL=$(ls "$C"/wheels/*.whl)
uv pip install -q --python "$PY" --force-reinstall --no-deps "$WHL" >> "$C/build-g2.log" 2>&1 || abort "wheel install"
WHLSHA=$(sha256sum "$WHL" | cut -d' ' -f1)
{ echo "harness=$HARNESS"; echo "warm=$WARMFULL (branch perf/l22-apex)"; echo "wheel=$WHL"; echo "$WHLSHA  $WHL"; rustc +1.99.0 -V; uv pip freeze --python "$PY"; } > "$O/build-info.txt"
st "wheel built $(basename "$WHL") sha256=$WHLSHA"
( cd /tmp && RAYON_NUM_THREADS=4 "$PY" "$IN/smoke2.py" ) > "$O/smoke.log" 2>&1 || abort "smoke, see $O/smoke.log"
st "smoke: $(grep -E 'recall|SMOKE' $O/smoke.log | tr '\n' ' ' | cut -c1-200)"
{ lscpu; date -u; free -m; uname -r; } > "$O/env.txt" 2>&1
lscpu | grep -o -w -E 'avx512vbmi|avx512_vnni|avx512bw' | sort -u | tr '\n' ' ' > "$O/cpuflags.txt"
CAP=$(systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 bash -c 'cat /sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/memory.max /sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/memory.swap.max' 2>&1 | tr '\n' ' ')
st "scope check: $CAP"
case "$CAP" in "8589934592 0 "*) ;; *) abort "scope not enforcing: $CAP";; esac
[ -d "$C/results-pre-s" ] || { [ -d "$W/results" ] && mv "$W/results" "$C/results-pre-s"; }

run_one(){ # label nprobe reorder knobs
  local L=$1 NP=$2 RO=$3 KN=${4:-} OUT=$O/$1
  mkdir -p "${OUT:?}"
  cd "$W" || abort "cd"
  git checkout -- "$CFG"
  /usr/bin/python3 -I - "$W/$CFG" "$NP" "$RO" <<'PY' || abort "config edit"
import sys
p,np,ro=sys.argv[1:4]
s=open(p).read()
i=s.index("msturing-30M-clustered:")
j=s.find("\n\n",i); j=len(s) if j<0 else j
b=s[i:j]
a1='"reorder":317'; a2='[{"nprobe":300}]'
assert b.count(a1)==1 and b.count(a2)==1, (b.count(a1),b.count(a2))
b=b.replace(a1,'"reorder":%s'%ro).replace(a2,'[{"nprobe":%s}]'%np)
open(p,"w").write(s[:i]+b+s[j:])
PY
  git diff > "$OUT/harness-diff.txt"; git status --short > "$OUT/harness-status.txt"
  [ "$(git diff --numstat | awk '{a+=$1;d+=$2} END{print a","d}')" = "2,2" ] || abort "unexpected diff size"
  st "[$L] config edited: $(grep -E '^[+-] ' "$OUT/harness-diff.txt" | tr -s ' ' | tr '\n' '|')"
  rm -rf results
  for i in $(seq 1 60); do Ld=$(cut -d' ' -f1 /proc/loadavg); awk -v l="$Ld" 'BEGIN{exit !(l<0.5)}' && break; sleep 10; done
  ps -eo pid,pcpu,cmd --sort=-pcpu | head -6 | cut -c1-150 > "$OUT/idle-check.txt"; echo "load=$(cat /proc/loadavg)" >> "$OUT/idle-check.txt"
  Ld=$(cut -d' ' -f1 /proc/loadavg); awk -v l="$Ld" 'BEGIN{exit !(l<0.5)}' || abort "machine not idle before $L: load=$Ld"
  export RAYON_NUM_THREADS=8 OMP_NUM_THREADS=8 WARM_LOG_RETRAIN=1; unset WARM_MODE; if [ -n "$KN" ]; then export WARM_PARTITION_KNOBS="$KN"; else unset WARM_PARTITION_KNOBS; fi; echo "knobs=$KN" > "$OUT/knobs.txt"
  st "[$L] run start (knobs $KN nprobe $NP reorder $RO, 8 threads, taskset 0-7, nice 19, 8G scope no swap, load=$(cut -d' ' -f1 /proc/loadavg))"
  systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
    nice -n 19 taskset -c 0-7 /usr/bin/time -v -o "$OUT/time.txt" \
    "$PY" -u run.py --nodocker --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered \
    --runbook_path neurips23/runbooks/final_runbook.yaml \
    > "$OUT/harness-stdout.log" 2> >(/usr/bin/python3 -I "$IN/ts.py" > "$OUT/phase-stderr.log")
  echo "run_exit=$?" > "$OUT/rc.txt"
  st "[$L] run end $(cat "$OUT/rc.txt")"
  sleep 2
  "$PY" data_export.py --output "$OUT/export.csv" > "$OUT/data_export.log" 2>&1
  st "[$L] export exit=$?"
  mv results "$OUT/results"
  /usr/bin/python3 -I "$IN/testb_report.py" "$W/neurips23/runbooks/final_runbook.yaml" "$C/pin/out" "$OUT" > "$OUT/report.txt" 2>&1
  st "[$L] REPORT $(grep -E '^(TEST B|GATES|mean recall|wall)' "$OUT/report.txt" | tr '\n' ' ' | cut -c1-400)"
  git checkout -- "$CFG"
}
for spec in "${SPECS[@]}"; do IFS=: read -r L NP RO KN <<< "$spec"; run_one "$L" "$NP" "$RO" "$KN"; done
( cd "$W" && git checkout -- "$CFG"; git status --short > "$O/harness-status-final.txt"; git diff --stat >> "$O/harness-status-final.txt" )
date -u > "$C/DONE-G2"
