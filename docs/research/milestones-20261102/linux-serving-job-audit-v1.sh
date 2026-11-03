#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' '748cb56e5a983d0fc90ddee9d0641013dd1daf2789131f35de1752894f7214a6  serving-job-audit-v1.tar' | sha256sum -c -
mkdir serving-job-audit-v1
tar -xf serving-job-audit-v1.tar -C serving-job-audit-v1
cd serving-job-audit-v1
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 PYTHONPATH=benchmarks/public
trap 'result=$?; if test "$result" != 0; then printf "%s\n" "{\"complete\":false,\"exitCode\":$result,\"synthetic\":true,\"claimQualified\":false}" > "$task_root/evidence/serving-job-audit-v1-finished.json"; fi' EXIT
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
import time
root=Path.home()/'warm-milestones-20261103'
deadline=time.monotonic()+86400
while not (root/'evidence/qualification-check-v1-finished.json').exists():
    if time.monotonic()>deadline:
        raise TimeoutError('qualification development checks have not finished within twenty-four hours')
    time.sleep(60)
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen serving auditor changed: '+name)
PY
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python -m unittest test_audit_serving_job > "$task_root/evidence/serving-job-audit-v1-tests.log" 2>&1
for variant in warm hnsw ivfflat faiss lucene; do
    "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/audit_serving_job.py --synthetic --root "$task_root/evidence/qualification-check-$variant-v1" --output "$task_root/evidence/serving-job-audit-$variant-v1" > "$task_root/evidence/serving-job-audit-$variant-v1.log" 2>&1
done
"$task_root/tools/uv/bin/uv" run --no-project python - <<'PY'
import hashlib
import json
from pathlib import Path
source=json.loads(Path('source.json').read_text())
for name, expected in source['files'].items():
    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected:
        raise RuntimeError('frozen serving auditor changed: '+name)
PY
printf '%s\n' '{"complete":true,"synthetic":true,"claimQualified":false}' > "$task_root/evidence/serving-job-audit-v1-finished.json"
