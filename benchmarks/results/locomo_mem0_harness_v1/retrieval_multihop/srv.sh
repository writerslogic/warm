#!/bin/sh
# Start the pinned pilot server on a private copy of the ingested store; no provider keys in the environment.
env -i HOME=$HOME PATH=$PATH $WARM_BENCH_TARGET/locomo/release/pilot-memsrv --no-auth --bind 127.0.0.1:18777 --data-dir $WARM_BENCH_TARGET/mh2store/data --input-dim 1536 --max-top-k ${MAXK:-200} --max-queue 1000 > $WARM_BENCH_DATA/multihop2/server.log 2>&1 &
echo $! > $WARM_BENCH_DATA/multihop2/server.pid
