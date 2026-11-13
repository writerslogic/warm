#!/usr/bin/env bash
set -euo pipefail
cd /mnt/warm-pr898
exec > >(tee live.log) 2>&1
test "$(date -u +%s)" -lt "$(date -u -d '2026-11-10 03:25:00' +%s)"
sha256sum -c SHA256SUMS
mkdir -p inputs index out
tar -xf dataset.tar -C inputs
docker load -i image.tar.gz
expected=$(python3 -c 'import json; print(json.load(open("cloud-image.json"))[0]["Id"])')
test "$(docker image inspect warm-pr898-live --format '{{.Id}}')" = "$expected"
lscpu > hardware.txt
uname -a >> hardware.txt
date -u +%FT%TZ > started-utc
archive() {
  rc=$?
  trap - EXIT
  date -u +%FT%TZ > finished-utc
  printf '%s\n' "$rc" > exit-code
  if test -f index/vector_bench_test/index.warm; then
    sha256sum index/vector_bench_test/index.warm > index-sha256.txt
    stat index/vector_bench_test/index.warm > index-stat.txt
  fi
  tar -czf results.tar.gz out live.log hardware.txt started-utc finished-utc exit-code \
    cloud-manifest.json cloud-image.json SHA256SUMS cloud-live.sh \
    --ignore-failed-read index-sha256.txt index-stat.txt
  sha256sum results.tar.gz > results.sha256
  mkdir -p /var/lib/warm-pr898
  cp results.tar.gz results.sha256 /var/lib/warm-pr898/
  sync
  exit "$rc"
}
trap archive EXIT
for spec in \
  sdk064:0:fresh:azure-sdk064-b0-r1 \
  main:0:reuse:azure-main-b0-r1 \
  final:0:reuse:azure-final-b0-r1 \
  final:1:reuse:azure-final-b1-r1 \
  shared:1:reuse:azure-shared-b1-r1 \
  main:1:reuse:azure-main-b1-r1 \
  sdk064:1:reuse:azure-sdk064-b1-r1 \
  sdk064:1:reuse:azure-sdk064-b1-r2 \
  main:1:reuse:azure-main-b1-r2 \
  shared:1:reuse:azure-shared-b1-r2 \
  final:1:reuse:azure-final-b1-r2 \
  final:0:reuse:azure-final-b0-r2 \
  main:0:reuse:azure-main-b0-r2 \
  sdk064:0:reuse:azure-sdk064-b0-r2; do
  IFS=: read -r arm batch mode label <<< "$spec"
  sync
  echo 3 > /proc/sys/vm/drop_caches
  date -u +%FT%TZ
  echo "RUN $spec"
  docker run --rm --name "warm-pr898-$label" --cpus=8 --cpuset-cpus=0-7 \
    --memory=16g --memory-swap=16g \
    -v "$PWD/inputs/dataset:/opt/warm-pr898/dataset:ro" \
    -v "$PWD/index:/opt/warm-pr898/index" -v "$PWD/out:/out" \
    warm-pr898-live "$arm" "$batch" "$mode" "$label"
done
