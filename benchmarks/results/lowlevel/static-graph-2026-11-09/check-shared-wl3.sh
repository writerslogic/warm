#!/usr/bin/env bash
set -euo pipefail
root=/opt/warm-pr898
cd "$root"
test -f native/NARROW-DONE
exec > >(tee native/shared-check.log) 2>&1
docker run --rm --memory=20g --memory-swap=20g --entrypoint /bin/bash \
  -v "$PWD/native:/work" warm-az1-streaming-control -lc '
set -euo pipefail
cd /home/app/warm
git fetch origin 431722cfba78b02666d1d1ce6004621e23fc71a2
git checkout --detach FETCH_HEAD
git apply /work/shared-source.patch
export CARGO_ENCODED_RUSTFLAGS=""
rustup component add clippy > /work/shared-clippy-install.log 2>&1
cargo clippy --locked --features python,local-embed-download --lib --target-dir /work/target -- -D warnings > /work/shared-clippy.log 2>&1
cargo test --locked --release --features experimental core::qgraph --lib --target-dir /work/target > /work/shared-qgraph.log 2>&1
cargo build --locked --release --features python,local-embed-download --lib --target-dir /work/target > /work/shared-build.log 2>&1
mkdir -p /work/shared-module
cp /work/target/release/libwriterslogic_warm.so /work/shared-module/writerslogic_warm.abi3.so
sha256sum /work/shared-module/writerslogic_warm.abi3.so > /work/shared-module-sha256.txt
'
PYTHONPATH="$PWD/native/shared-module" venv/bin/python -m pytest \
  native/shared-tests/test_vector_index.py native/shared-tests/test_specialized_index.py \
  native/shared-tests/test_shared_index.py -q > native/shared-python.log 2>&1
context="$root/cloud-context"
mkdir -p "$context/main-module"
cp -a native/control-module/. "$context/main-module/"
cp -a native/shared-module/. "$context/final-module/"
git -C harness archive 4c6bdb566423d39afc2287bd27c4a3787eb80113 | tar -x -C "$context/harness"
(cd "$context/harness" && git apply "$root/native/shared-adapter.patch")
cp native/cloud.Dockerfile "$context/Dockerfile"
cp native/cloud-run-one.sh "$context/run-one.sh"
cp native/sealed_index_run.py "$context/"
cp native/cloud-manifest.json "$context/manifest.json"
PYTHONPATH="$context/harness" venv/bin/python -m pytest "$context/harness/tests/test_warm.py" "$context/harness/tests/test_warm_frontend.py" -q > native/shared-adapter-tests.log 2>&1
docker build -t warm-pr898-shared-live "$context" > native/shared-image-build.log 2>&1
docker image inspect warm-pr898-shared-live > native/shared-image.json
for spec in final:local-shared-heap-r1 shared:local-shared-map-r1 shared:local-shared-map-r2 final:local-shared-heap-r2; do
  IFS=: read -r arm label <<< "$spec"
  docker run --rm --cpus=8 --cpuset-cpus=0-7 --memory=16g --memory-swap=16g \
    -v "$root/dataset:$root/dataset:ro" -v "$root/cloud-local/index:$root/index" \
    -v "$root/cloud-local/out:/out" warm-pr898-shared-live "$arm" 1 reuse "$label"
done
date -u +%FT%TZ > native/SHARED-DONE
