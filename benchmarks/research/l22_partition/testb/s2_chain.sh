#!/usr/bin/env bash
# L22 test B on wl-03: setup -> wheel (python,experimental) -> smoke -> final_runbook with WARM_MODE=partition -> export -> report.
set -uo pipefail
export PATH=$HOME/.cargo/bin:/home/linuxbrew/.linuxbrew/bin:$PATH
S0=$HOME/bigann/s0; S1C=$HOME/bigann/s1c; C=$HOME/bigann/s2; LOG=$HOME/bigann/logs; ST=$LOG/s2-status.txt
W=$C/bab; PY=$C/venv/bin/python; OUT=$C/out; IN="/mnt/c/Users/David Condrey/s2in"
H=5d6225a00c763cb2eb67b46585009f353f7c9651
st(){ echo "$(date -u +%FT%TZ) $*" >> "$ST"; }
abort(){ st "ABORT $*"; echo "$*" > "$C/ABORT"; exit 1; }
mkdir -p "$C" "$OUT" "$LOG"; rm -f "$C/ABORT" "$C/DONE"
st "chain-start pid=$$"
cp "$IN/src.tgz" "$IN/partition-mode.patch" "$IN/smoke2.py" "$IN/testb_report.py" "$IN/SHA256SUMS" "$C/" || abort "copy inputs"
( cd "$C" && sha256sum -c SHA256SUMS ) >> "$ST" 2>&1 || abort "input sha"
rm -rf "${C:?}/src" && mkdir -p "$C/src" && tar xzf "$C/src.tgz" -C "$C/src" || abort "tar x"
grep -q 'name = "PartitionIndex"' "$C/src/src/bindings/python/partition_index.rs" || abort "binding not in source"
cp "$IN/COMMIT" "$C/build-info.txt"
if [ ! -x "$PY" ]; then
  nice -n 19 taskset -c 8-21 uv venv -q --python 3.10 "$C/venv" || abort "uv venv"
  nice -n 19 taskset -c 8-21 uv pip install -q --python "$PY" -r "$S1C/requirements-s1.txt" || abort "reqs"
fi
if [ ! -d "$W" ]; then git -C "$HOME/bigann/bab" worktree add --detach "$W" $H >> "$LOG/s2-setup.log" 2>&1 || abort "worktree add"; fi
cd "$W" && [ "$(git rev-parse HEAD)" = "$H" ] || abort "bab HEAD"
git checkout -q -- neurips23/streaming/warm/warm.py
git apply --check "$C/partition-mode.patch" && git apply "$C/partition-mode.patch" || abort "patch"
mkdir -p "$W/data"; [ -d "$W/data/MSTuring-30M-clustered" ] || cp -al "$S0/bab/data/MSTuring-30M-clustered" "$W/data/" || abort "data hardlink"
st "wheel build start ($(rustc +1.99.0 -V))"
( cd "$C/src" && . "$C/venv/bin/activate" && RUSTUP_TOOLCHAIN=1.99.0 CARGO_TARGET_DIR="$C/target" CARGO_BUILD_JOBS=8 \
  nice -n 19 maturin build --release --features python,experimental -i "$PY" -o "$C/wheels" ) > "$C/build.log" 2>&1 || abort "wheel build"
WHL=$(ls -t "$C"/wheels/*.whl | head -1)
uv pip install -q --python "$PY" --force-reinstall --no-deps "$WHL" >> "$C/build.log" 2>&1 || abort "wheel install"
{ echo "wheel=$WHL"; sha256sum "$WHL"; rustc +1.99.0 -V; } >> "$C/build-info.txt"
( cd /tmp && RAYON_NUM_THREADS=4 nice -n 19 taskset -c 8-11 "$PY" "$C/smoke2.py" ) > "$C/smoke.log" 2>&1 || abort "smoke, see $C/smoke.log"
st "smoke: $(grep -E 'recall|SMOKE' $C/smoke.log | tr '\n' ' ' | cut -c1-200)"
{ lscpu | head -20; date -u; free -g; uname -r; } > "$OUT/env.txt" 2>&1
{ git rev-parse HEAD; git status --short; git diff --stat; } > "$OUT/harness-status.txt" 2>&1
cp "$C/build-info.txt" "$C/smoke.log" "$OUT/"
rm -rf "${W:?}/results"
export RAYON_NUM_THREADS=8 OMP_NUM_THREADS=8 WARM_MODE=partition
st "run start (WARM_MODE=partition nprobe 250, 8 threads, taskset 0-7, nice 19, 8G scope no swap)"
flock -w 14400 "$HOME/hms-timing.lock" systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 \
  nice -n 19 taskset -c 0-7 /usr/bin/time -v -o "$OUT/time.txt" \
  "$PY" -u run.py --nodocker --neurips23track streaming --algorithm warm --dataset msturing-30M-clustered \
  --runbook_path neurips23/runbooks/final_runbook.yaml \
  > "$OUT/harness-stdout.log" 2> >(/usr/bin/python3 -I "$S0/ts.py" > "$OUT/phase-stderr.log")
echo "run_exit=$?" > "$OUT/rc.txt"
st "run end $(cat "$OUT/rc.txt")"
sleep 2
"$PY" data_export.py --output "$OUT/export.csv" > "$OUT/data_export.log" 2>&1
st "export exit=$?"
cp -r results "$OUT/results" 2>/dev/null
/usr/bin/python3 -I "$C/testb_report.py" "$W/neurips23/runbooks/final_runbook.yaml" "$S0/out" "$OUT" > "$OUT/report.txt" 2>&1
st "REPORT $(grep -E '^(TEST B|GATES|mean recall)' "$OUT/report.txt" | tr '\n' ' ' | cut -c1-400)"
date -u > "$C/DONE"
