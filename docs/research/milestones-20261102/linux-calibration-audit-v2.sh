#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
cd calibration-audit-driver-v2
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
export PYTHONPATH=benchmarks/public
trap 'result=$?; if test "$result" != 0; then printf "%s\n" "{\"complete\":false,\"exitCode\":$result,\"heldOutVectorsOpened\":false}" > "$task_root/evidence/calibration-audit-v2-finished.json"; fi' EXIT
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
import time
root=Path.home()/'warm-milestones-20261103'
deadline=time.monotonic()+86400
while not (root/'evidence/qualification-check-v1-finished.json').exists():
    if time.monotonic()>deadline:
        raise TimeoutError('qualification development checks have not finished after twenty-four hours')
    time.sleep(60)
for entry in Path('/proc').iterdir():
    if not entry.name.isdigit():
        continue
    try:
        command=(entry/'cmdline').read_bytes()
    except FileNotFoundError:
        continue
    if b'benchmarks/public/matched_server/calibrate.py' in command:
        raise RuntimeError('calibration process remains active')
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen calibration auditor changed: '+name)
PY
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python -m unittest test_audit_calibration > "$task_root/evidence/calibration-audit-v2-tests.log" 2>&1
for dataset in coco-i2i-512-angular deep-image-96-angular; do
    for engine in warm pgvector opensearch; do
        version=v2
        if test "$dataset" = coco-i2i-512-angular && test "$engine" = pgvector; then
            version=v1
        fi
        "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/audit_calibration.py --calibration "$task_root/evidence/calibration-$engine-$dataset-$version" --inputs "$task_root/performance-calibration-inputs" --output "$task_root/evidence/calibration-audit-$engine-$dataset-v2" > "$task_root/evidence/calibration-audit-$engine-$dataset-v2.log" 2>&1
    done
done
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen calibration auditor changed: '+name)
PY
printf '%s\n' '{"complete":true,"heldOutVectorsOpened":false}' > "$task_root/evidence/calibration-audit-v2-finished.json"
