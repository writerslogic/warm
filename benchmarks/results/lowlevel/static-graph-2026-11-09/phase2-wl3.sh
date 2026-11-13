#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
root=/opt/warm-pr898
cd "$root"
test -f native/BATCH-DONE
test -z "$(git -C harness status --porcelain)"
git -C harness fetch https://github.com/dcondrey/VectorDBBench.git 4c6bdb566423d39afc2287bd27c4a3787eb80113
git -C harness checkout --detach FETCH_HEAD
venv/bin/python -m pytest harness/tests/test_warm.py harness/tests/test_warm_frontend.py -q
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
export DATASET_LOCAL_DIR="$root/dataset"
export RESULTS_LOCAL_DIR="$root/results/sdk064-batch-fresh"
export LOG_FILE="$root/native/harness-sdk064-batch-fresh.log"
export WARM_QGRAPH_BATCH_EST=1
mkdir -p "$RESULTS_LOCAL_DIR"
/usr/bin/time -v venv/bin/vectordbbench warm --case-type Performance768D1M \
  --task-label wl3-sdk064-batch-fresh --db-label wl3-sdk064-batch \
  --path "$root/index-064-new" --num-concurrency 1,5,10 \
  --k 100 --ef-search 150 --m 16 --ef-construction 300 --insert-batch-size 1000 \
  > "$root/native/run-sdk064-batch-fresh.log" 2>&1
sha256sum index-064/vector_bench_test/index.warm index-064-new/vector_bench_test/index.warm > native/index-rebuild-sha256.txt
unset WARM_QGRAPH_BATCH_EST
docker run --rm --memory=20g --memory-swap=20g --entrypoint /bin/bash \
  -v "$root/native:/work" warm-az1-streaming-control -lc '
set -euo pipefail
cd /home/app/warm
git fetch origin 431722cfba78b02666d1d1ce6004621e23fc71a2
git checkout --detach FETCH_HEAD
git apply /work/integrated.patch
cargo test --locked --release --features experimental core::qgraph --lib --target-dir /work/target > /work/integrated-qgraph-tests.log 2>&1
for tier in scalar avx2 avxvnni; do
  WARM_QGRAPH_KERNEL=$tier cargo test --locked --release --features experimental long_i8_dots_widen_before_accumulation_overflows --lib --target-dir /work/target > "/work/integrated-wide-$tier.log" 2>&1
done
cargo build --locked --release --features python,local-embed-download --lib --target-dir /work/target > /work/integrated-build.log 2>&1
mkdir -p /work/integrated-module
cp /work/target/release/libwriterslogic_warm.so /work/integrated-module/writerslogic_warm.abi3.so
sha256sum /work/integrated-module/writerslogic_warm.abi3.so > /work/integrated-module-sha256.txt
' > native/integrated-build-driver.log 2>&1
PYTHONPATH="$root/native/integrated-module" venv/bin/python -m pytest native/test_vector_index-integrated.py -q > native/integrated-python-tests.log 2>&1
for spec in sdk064:0:4 integrated:0:1 integrated:1:1 integrated:1:2 integrated:0:2 sdk064:0:5 sdk064:1:1 sdk064:1:2; do
  IFS=: read -r arm batch round <<< "$spec"
  if [[ $arm == sdk064 ]]; then unset PYTHONPATH; else export PYTHONPATH="$root/native/$arm-module"; fi
  export WARM_QGRAPH_BATCH_EST=$batch
  export RESULTS_LOCAL_DIR="$root/results/phase2-$arm-b$batch-r$round"
  export LOG_FILE="$root/native/harness-phase2-$arm-b$batch-r$round.log"
  mkdir -p "$RESULTS_LOCAL_DIR"
  date -u +%FT%TZ
  echo "RUN $arm batch=$batch round=$round"
  /usr/bin/time -v venv/bin/vectordbbench warm --case-type Performance768D1M \
    --task-label "wl3-phase2-$arm-b$batch-r$round" --db-label "wl3-$arm-b$batch" \
    --path "$root/index-064-new" --skip-drop-old --skip-load --num-concurrency 1,5,10 \
    --k 100 --ef-search 150 --m 16 --ef-construction 300 --insert-batch-size 1000 \
    > "$root/native/run-phase2-$arm-b$batch-r$round.log" 2>&1
done
date -u +%FT%TZ > "$root/native/PHASE2-DONE"
