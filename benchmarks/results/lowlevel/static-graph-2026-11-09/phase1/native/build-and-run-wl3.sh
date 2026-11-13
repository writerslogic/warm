#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
root=/opt/warm-pr898
cd "$root/native"
docker run --rm --memory=20g --memory-swap=20g --entrypoint /bin/bash \
  -v "$root/native:/work" warm-az1-streaming-control -lc '
set -euo pipefail
cd /home/app/warm
git fetch origin 431722cfba78b02666d1d1ce6004621e23fc71a2
git checkout --detach FETCH_HEAD
for arm in control paired prefetch quality; do
  git restore src
  case "$arm" in
    paired) git apply /work/paired.patch ;;
    prefetch) git apply /work/prefetch-only.patch ;;
    quality) git apply /work/quality-source.patch ;;
  esac
  mkdir -p "/work/$arm-module"
  cargo build --locked --release --features python,local-embed-download --lib --target-dir /work/target > "/work/build-$arm.log" 2>&1
  cp /work/target/release/libwriterslogic_warm.so "/work/$arm-module/writerslogic_warm.abi3.so"
  sha256sum "/work/$arm-module/writerslogic_warm.abi3.so"
done
git apply /work/quality-python-tests.patch
cargo test --locked --release --no-default-features core::qgraph::rerank --lib --target-dir /work/target > /work/x86-final-kernels.log 2>&1
cargo test --locked --release --no-default-features extreme_angular_queries_preserve_complete_order --lib --target-dir /work/target >> /work/x86-final-kernels.log 2>&1
' > "$root/native/build.log" 2>&1
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd "$root"
PYTHONPATH="$root/native/quality-module" venv/bin/python -m pytest native/test_vector_index.py -q > native/python-tests.log 2>&1
export DATASET_LOCAL_DIR="$root/dataset"
for spec in sdk064:2 control:1 paired:1 prefetch:1 quality:1 quality:2 prefetch:2 paired:2 control:2 sdk064:3; do
  arm=${spec%:*}; round=${spec#*:}
  if [[ $arm == sdk064 ]]; then unset PYTHONPATH; else export PYTHONPATH="$root/native/$arm-module"; fi
  export RESULTS_LOCAL_DIR="$root/results/$arm-$round"
  export LOG_FILE="$root/native/harness-$arm-$round.log"
  mkdir -p "$RESULTS_LOCAL_DIR"
  date -u +%FT%TZ
  echo "RUN $arm $round"
  venv/bin/python -c 'import writerslogic_warm as w; print(w.__file__)'
  /usr/bin/time -v venv/bin/vectordbbench warm --case-type Performance768D1M \
    --task-label "wl3-$arm-r$round-ef150" --db-label "wl3-$arm" \
    --path "$root/index-064" --skip-drop-old --skip-load --num-concurrency 1,5,10 \
    --k 100 --ef-search 150 --m 16 --ef-construction 300 --insert-batch-size 1000 \
    > "$root/native/run-$arm-$round.log" 2>&1
done
date -u +%FT%TZ > "$root/native/BENCHMARK-DONE"
