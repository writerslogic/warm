#!/bin/sh
set -eu

test "$(uname -m)" = aarch64
test "$(id -u)" -ne 0
task_root="$PWD/warm-milestones-20261103"
task_tools="$task_root/tools"
mkdir -p "$task_tools" "$task_root/evidence" "$task_root/tarballs" "$task_root/consumer"
printf '%s  %s\n' ddfc3d5fa66afb4317b66742e6724691c489ed9c2f0071ca9e2194d398cc350a "$PWD/warm-source.tar" | sha256sum -c -
mkdir "$task_root/source"
tar -xf "$PWD/warm-source.tar" -C "$task_root/source"
printf '%s  %s\n' 8194367e5c1775bc7710a870cda04027f36b33bf3150485fd7457a94aef8fdb6 "$PWD/cargo-targets.tar" | sha256sum -c -
tar -xf "$PWD/cargo-targets.tar" -C "$task_root/source"

curl -fsSL --connect-timeout 15 --max-time 180 https://static.rust-lang.org/rustup/dist/aarch64-unknown-linux-gnu/rustup-init -o "$task_tools/rustup-init"
printf '%s  %s\n' a9c168dbea81a84671c6b7a5c1795cb0eb699020110d107ddfca427bd797e697 "$task_tools/rustup-init" | sha256sum -c -
chmod 700 "$task_tools/rustup-init"
"$task_tools/rustup-init" -y --profile minimal --default-toolchain 1.96.0 --no-modify-path > "$task_root/evidence/rust-install.log" 2>&1

curl -fsSL --connect-timeout 15 --max-time 180 https://nodejs.org/dist/v22.23.4/node-v22.23.4-linux-arm64.tar.xz -o "$task_tools/node.tar.xz"
printf '%s  %s\n' 6287aa46b6c375d816e63ae66d024fbbfd70404825aabdfdf898a970871d2f4b "$task_tools/node.tar.xz" | sha256sum -c -
tar -xJf "$task_tools/node.tar.xz" -C "$task_tools"
export PATH="$task_tools/node-v22.23.4-linux-arm64/bin:$HOME/.cargo/bin:$PATH"
python3 -m venv "$task_tools/uv"
"$task_tools/uv/bin/pip" install 'uv==0.12.10' > "$task_root/evidence/uv-install.log" 2>&1

cd "$task_root/source"
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="$task_root/target"
export CARGO_PROFILE_RELEASE_STRIP=none
export RAYON_NUM_THREADS=2
rustc --version > "$task_root/evidence/versions.txt"
node --version >> "$task_root/evidence/versions.txt"
npm --version >> "$task_root/evidence/versions.txt"
"$task_tools/uv/bin/uv" --version >> "$task_root/evidence/versions.txt"
cargo build --locked --release --lib --features node-api,security > "$task_root/evidence/node-build.log" 2>&1
cp "$task_root/target/release/libwriterslogic_warm.so" warm-sdk.linux-arm64-gnu.node
npm ci --ignore-scripts --no-audit --no-fund > "$task_root/evidence/node-development-install.log" 2>&1
npm pack --json --ignore-scripts --pack-destination "$task_root/tarballs" > "$task_root/evidence/node-pack.json"
cd "$task_root/consumer"
printf '%s\n' '{"name":"warm-linux-arm64-consumer","private":true}' > package.json
npm install --ignore-scripts --omit=optional --no-audit --no-fund "$task_root/tarballs/warm-sdk-0.6.5.tgz" > "$task_root/evidence/node-fresh-install.log" 2>&1
cp -R "$task_root/source/tests" node_modules/warm-sdk/tests
cp -R "$task_root/source/examples" node_modules/warm-sdk/examples
cd node_modules/warm-sdk
npm test > "$task_root/evidence/node-api-tests.log" 2>&1
cd "$task_root/source"
npm run test:types > "$task_root/evidence/node-type-tests.log" 2>&1
npm run test:package > "$task_root/evidence/node-package-tests.log" 2>&1
node benchmarks/public/node_recovery.cjs "$task_root/consumer/node_modules/warm-sdk" "$task_root/evidence/node-recovery" > "$task_root/evidence/node-recovery.log" 2>&1
cargo build --locked --release --bin warm-server --features server,security > "$task_root/evidence/server-build.log" 2>&1
sha256sum warm-sdk.linux-arm64-gnu.node "$task_root/target/release/warm-server" "$task_root/tarballs/warm-sdk-0.6.5.tgz" > "$task_root/evidence/artifact-sha256.txt"
printf '%s\n' complete > "$task_root/evidence/bootstrap-finished.txt"
