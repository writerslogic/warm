#!/bin/zsh
# usage: build.sh SIZE...   builds competitive_text per block size, copies to /Volumes/A/.hms-bin-l3
export CARGO_TARGET_DIR=/Volumes/A/.hms-target-l3
export CARGO_PROFILE_DIST_LTO=false
export CARGO_INCREMENTAL=0
cd /Volumes/A/.hms-wt-l3
for s in "$@"; do
  start=$(date +%s)
  WARM_POSTINGS_BLOCK_LEN=$s nice -n 15 cargo build --profile dist --features cli,mimalloc --example competitive_text > /private/tmp/claude-501/-Volumes-A-holographic-memory/1631bb91-3119-442e-b4c1-a094c160e609/scratchpad/build-$s.log 2>&1
  rc=$?
  cp $CARGO_TARGET_DIR/dist/examples/competitive_text /Volumes/A/.hms-bin-l3/competitive_text-$s
  echo "size $s rc $rc secs $(( $(date +%s) - start ))" >> /private/tmp/claude-501/-Volumes-A-holographic-memory/1631bb91-3119-442e-b4c1-a094c160e609/scratchpad/build-times.log
done
