#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' 'aad7bb2083ceb8c7fe22170eb4ed70319a16958b80fc49eea0d60a231cc46d6b  measurement-driver-v1.tar' 'e10107257731bb3e0c18c8ef4bf43e6fb606e0eb1ef4cc801c3a6e55356d5361  calibration-qualified-source/warm-server' | sha256sum -c -
mkdir measurement-driver-v1
tar -xf measurement-driver-v1.tar -C measurement-driver-v1
export WARM_BENCH_WARM_SERVER="$task_root/calibration-qualified-source/warm-server"
export WARM_BENCH_PG_BIN="$task_root/comparators/postgres/bin"
export WARM_MATCHED_OPENSEARCH_HOME="$task_root/comparators/opensearch-3.9.0"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd measurement-driver-v1
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
import time
root=Path.home()/'warm-milestones-20261103'
deadline=time.monotonic()+43200
while not (root/'evidence/calibration-v2-finished.txt').exists():
    if time.monotonic()>deadline:
        raise TimeoutError('calibration has not completed after twelve hours')
    time.sleep(60)
for entry in Path('/proc').iterdir():
    if not entry.name.isdigit():
        continue
    try:
        command=(entry/'cmdline').read_bytes()
    except FileNotFoundError:
        continue
    if b'benchmarks/public/matched_server/calibrate.py' in command:
        raise RuntimeError('calibration process remains active despite terminal marker')
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen measurement source changed: '+name)
PY
PYTHONPATH=benchmarks/public/matched_server "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python -m unittest test_protocol test_measure test_claim_gate > "$task_root/evidence/measurement-v1-tests.log" 2>&1
for variant in warm hnsw ivfflat faiss lucene; do
    result=0
    taskset -c 0-3 "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/check_measure.py --variant "$variant" --output "$task_root/evidence/measurement-$variant-v1" > "$task_root/evidence/measurement-$variant-v1.log" 2>&1 || result=$?
    printf '%s %s\n' "$variant" "$result" >> "$task_root/evidence/measurement-v1-exit-codes.txt"
    if test "$result" != 0; then
        printf '%s\n' '{"complete":false,"synthetic":true,"benchmarkClaim":false}' > "$task_root/evidence/measurement-v1-finished.json"
        exit "$result"
    fi
done
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen measurement source changed: '+name)
PY
printf '%s\n' '{"complete":true,"synthetic":true,"benchmarkClaim":false}' > "$task_root/evidence/measurement-v1-finished.json"
