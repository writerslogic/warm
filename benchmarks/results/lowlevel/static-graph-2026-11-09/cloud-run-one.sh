#!/usr/bin/env bash
set -euo pipefail
root=/opt/warm-pr898
cd "$root"
arm=${1:?arm}
batch=${2:?batch}
mode=${3:?mode}
label=${4:?label}
case "$arm" in
  sdk064) unset PYTHONPATH ;;
  final|main) export PYTHONPATH="$root/native/$arm-module" ;;
  shared) export PYTHONPATH="$root/native/final-module:$root" ;;
  *) exit 64 ;;
esac
case "$batch" in 0|1) ;; *) exit 64 ;; esac
case "$label" in *[!a-zA-Z0-9_-]*|'') exit 64 ;; esac
export WARM_QGRAPH_BATCH_EST="$batch"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
export DATASET_LOCAL_DIR="$root/dataset"
export RESULTS_LOCAL_DIR="/out/$label/results"
export LOG_FILE="/out/$label/harness.log"
mkdir -p "$RESULTS_LOCAL_DIR"
finish() {
  rc=$?
  for file in memory.peak memory.events cpu.stat; do
    cat "/sys/fs/cgroup/$file" > "/out/$label/$file" || true
  done
  printf '%s\n' "$rc" > "/out/$label/exit-code"
  date -u +%FT%TZ > "/out/$label/finished-utc"
  exit "$rc"
}
trap finish EXIT
date -u +%FT%TZ > "/out/$label/started-utc"
args=()
case "$mode" in fresh) ;; reuse) args=(--skip-drop-old --skip-load) ;; *) exit 64 ;; esac
command=("$root/venv/bin/vectordbbench" warm \
  --case-type Performance768D1M --task-label "$label" --db-label "$arm-b$batch" \
  --path "$root/index" --num-concurrency 1,5,10 --k 100 --ef-search 150 \
  --m 16 --ef-construction 300 --insert-batch-size 1000 "${args[@]}")
if [[ "$arm" == shared ]]; then
  command=("$root/venv/bin/python" "$root/sealed_index_run.py" \
    --index "$root/index/vector_bench_test/index.warm" --receipt "/out/$label/snapshot.json" -- "${command[@]}")
fi
/usr/bin/time -v "${command[@]}" \
  > "/out/$label/console.log" 2>&1
"$root/venv/bin/python" - "$RESULTS_LOCAL_DIR" <<'PY'
import json, pathlib, sys
paths = list(pathlib.Path(sys.argv[1]).glob('Warm/*.json'))
assert len(paths) == 1, paths
result = json.loads(paths[0].read_text())['results']
assert len(result) == 1, len(result)
metrics = result[0]['metrics']
assert metrics['conc_num_list'] == [1, 5, 10], metrics['conc_num_list']
assert len(metrics['conc_qps_list']) == 3 and min(metrics['conc_qps_list']) > 0
assert metrics['additional_parameters']['ground_truth']['query_count'] == 1000
assert 0 <= metrics['recall'] <= 1
print(json.dumps(metrics))
PY
