#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' 'fba17b05dc9c207c7acf896c2e7f35cda844754f7006fb008175a35a5639ac50  driver-v7.tar' 'e10107257731bb3e0c18c8ef4bf43e6fb606e0eb1ef4cc801c3a6e55356d5361  calibration-qualified-source/warm-server' | sha256sum -c -
mkdir matched-driver-v7
tar -xf driver-v7.tar -C matched-driver-v7
export WARM_BENCH_WARM_SERVER="$task_root/calibration-qualified-source/warm-server"
export WARM_BENCH_PG_BIN="$task_root/comparators/postgres/bin"
export WARM_MATCHED_OPENSEARCH_HOME="$task_root/comparators/opensearch-3.9.0"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd matched-driver-v7
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python -m unittest discover -s benchmarks/public/matched_server -p test_protocol.py > "$task_root/evidence/calibration-v2-protocol-tests.log" 2>&1
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
from pathlib import Path
import time
path = Path('/proc/25101/cmdline')
deadline = time.monotonic() + 28800
while path.exists():
    try:
        command = path.read_bytes()
    except FileNotFoundError:
        break
    if b'matched-driver-v6/' not in command or b'--engine\0pgvector' not in command:
        break
    if time.monotonic() > deadline:
        raise TimeoutError('prior PostgreSQL calibration still running after eight hours')
    time.sleep(30)
PY
for dataset in coco-i2i-512-angular deep-image-96-angular; do
    for engine in warm pgvector opensearch; do
        if test "$dataset" = coco-i2i-512-angular && test "$engine" = pgvector; then
            continue
        fi
        result=0
        taskset -c 0-3 "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/calibrate.py --engine "$engine" --dataset "$dataset" --inputs "$task_root/performance-calibration-inputs" --output "$task_root/evidence/calibration-$engine-$dataset-v2" --installation-evidence "$task_root/comparators/evidence" > "$task_root/evidence/calibration-$engine-$dataset-v2.log" 2>&1 || result=$?
        printf '%s %s %s\n' "$dataset" "$engine" "$result" >> "$task_root/evidence/calibration-v2-exit-codes.txt"
    done
done
printf '%s\n' complete > "$task_root/evidence/calibration-v2-finished.txt"
