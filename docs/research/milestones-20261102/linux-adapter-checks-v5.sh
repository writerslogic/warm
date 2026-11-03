#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' 'eec3aefcf93e3c7512a0add3faecde5972069f92aab19b915b581a64b1acaba5  driver-v5.tar' | sha256sum -c -
mkdir matched-driver-v5
tar -xf driver-v5.tar -C matched-driver-v5
export WARM_BENCH_WARM_SERVER="$task_root/target/release/warm-server"
export WARM_BENCH_PG_BIN="$task_root/comparators/postgres/bin"
export WARM_MATCHED_OPENSEARCH_HOME="$task_root/comparators/opensearch-3.9.0"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd matched-driver-v5
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/check_adapters.py --engine opensearch --candidate-k 32 --output "$task_root/evidence/adapter-opensearch-v5" > "$task_root/evidence/adapter-opensearch-v5.log" 2>&1
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/check_adapters.py --engine opensearch --opensearch-engine lucene --candidate-k 32 --output "$task_root/evidence/adapter-opensearch-lucene-v5" > "$task_root/evidence/adapter-opensearch-lucene-v5.log" 2>&1
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/check_adapters.py --engine pgvector --pg-index ivfflat --output "$task_root/evidence/adapter-pgvector-ivf-v5" > "$task_root/evidence/adapter-pgvector-ivf-v5.log" 2>&1
