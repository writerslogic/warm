#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' 'cc1835072e3da15e1f8fc44b046d888b95996b3a25895f8526e15e76449ed182  driver-v4.tar' | sha256sum -c -
mkdir matched-driver-v4
tar -xf driver-v4.tar -C matched-driver-v4
export WARM_BENCH_WARM_SERVER="$task_root/target/release/warm-server"
export WARM_BENCH_PG_BIN="$task_root/comparators/postgres/bin"
export WARM_MATCHED_OPENSEARCH_HOME="$task_root/comparators/opensearch-3.9.0"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd matched-driver-v4
for engine in opensearch; do
    "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/check_adapters.py --engine "$engine" --output "$task_root/evidence/adapter-$engine-v4" > "$task_root/evidence/adapter-$engine-v4.log" 2>&1
done
