#!/bin/sh
set -eu
task_root="$HOME/warm-milestones-20261103"
cd "$task_root"
printf '%s\n' 'a27c70fe1d29f7a40eeadc26456a0f21bb03cf5da7d95f9049022ad6b34c489c  driver-v6.tar' 'c8294bcfd641ba364ea730d3151ba715ff018f7c0a2163d8f087bdb098673f63  calibration-transfer.tar' 'e10107257731bb3e0c18c8ef4bf43e6fb606e0eb1ef4cc801c3a6e55356d5361  target/release/warm-server' | sha256sum -c -
mkdir matched-driver-v6 performance-calibration-inputs calibration-qualified-source
tar -xf driver-v6.tar -C matched-driver-v6
tar -xf calibration-transfer.tar -C performance-calibration-inputs
cp target/release/warm-server calibration-qualified-source/warm-server
cp "$0" calibration-qualified-source/launcher.sh
export WARM_BENCH_WARM_SERVER="$task_root/calibration-qualified-source/warm-server"
export WARM_BENCH_PG_BIN="$task_root/comparators/postgres/bin"
export WARM_MATCHED_OPENSEARCH_HOME="$task_root/comparators/opensearch-3.9.0"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
cd matched-driver-v6
"$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python -m unittest discover -s benchmarks/public/matched_server -p test_protocol.py > "$task_root/evidence/calibration-protocol-tests.log" 2>&1
for dataset in coco-i2i-512-angular deep-image-96-angular; do
    for engine in warm pgvector opensearch; do
        result=0
        taskset -c 0-3 "$task_root/tools/uv/bin/uv" run --project benchmarks/public/matched_server --frozen python benchmarks/public/matched_server/calibrate.py --engine "$engine" --dataset "$dataset" --inputs "$task_root/performance-calibration-inputs" --output "$task_root/evidence/calibration-$engine-$dataset-v1" --installation-evidence "$task_root/comparators/evidence" > "$task_root/evidence/calibration-$engine-$dataset-v1.log" 2>&1 || result=$?
        printf '%s %s %s\n' "$dataset" "$engine" "$result" >> "$task_root/evidence/calibration-v1-exit-codes.txt"
    done
done
printf '%s\n' complete > "$task_root/evidence/calibration-v1-finished.txt"
