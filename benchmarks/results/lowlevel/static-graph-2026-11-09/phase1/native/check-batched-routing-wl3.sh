#!/usr/bin/env bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
root=/opt/warm-pr898
while systemctl is-active --quiet warm-pr898-comparison.service; do sleep 15; done
test -f "$root/native/BENCHMARK-DONE"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 RAYON_NUM_THREADS=1
for spec in 0:1 1:1 1:2 0:2; do
  batch=${spec%:*}; round=${spec#*:}
  export WARM_QGRAPH_BATCH_EST=$batch
  "$root/venv/bin/python" "$root/native/compare-query-wl3.py" \
    --module "$root/native/quality-module/writerslogic_warm.abi3.so" \
    --output "$root/native/batch-$batch-$round.json" \
    > "$root/native/batch-$batch-$round.log" 2>&1
done
date -u +%FT%TZ > "$root/native/BATCH-DONE"
