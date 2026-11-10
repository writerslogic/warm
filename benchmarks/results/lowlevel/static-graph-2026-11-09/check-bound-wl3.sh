#!/usr/bin/env bash
set -euo pipefail
root=/opt/warm-pr898
cd "$root"
test -f native/SHARED-DONE
exec > >(tee native/bound-check.log) 2>&1
venv/bin/python -m pytest shared-protocol/tests/python/test_shared_snapshot_protocol.py -q > native/bound-protocol-tests.log 2>&1
context="$root/cloud-context"
git -C harness archive 4c6bdb566423d39afc2287bd27c4a3787eb80113 | tar -x -C "$context/harness"
(cd "$context/harness" && git apply "$root/native/shared-binding-adapter.patch")
cp native/cloud-run-one.sh "$context/run-one.sh"
cp shared-protocol/scripts/lowlevel/sealed_index_run.py "$context/"
PYTHONPATH="$context/harness" venv/bin/python -m pytest "$context/harness/tests/test_warm.py" "$context/harness/tests/test_warm_frontend.py" -q > native/bound-adapter-tests.log 2>&1
docker build -t warm-pr898-bound-live "$context" > native/bound-image-build.log 2>&1
docker image inspect warm-pr898-bound-live > native/bound-image.json
for spec in final:local-bound-heap-r1 shared:local-bound-map-r1 shared:local-bound-map-r2 final:local-bound-heap-r2; do
  IFS=: read -r arm label <<< "$spec"
  docker run --rm --cpus=8 --cpuset-cpus=0-7 --memory=16g --memory-swap=16g \
    -v "$root/dataset:$root/dataset:ro" -v "$root/cloud-local/index:$root/index" \
    -v "$root/cloud-local/out:/out" warm-pr898-bound-live "$arm" 1 reuse "$label"
done
date -u +%FT%TZ > native/BOUND-DONE
