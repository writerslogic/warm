#!/bin/sh
# Start the pinned pilot server on a private copy of the ingested store; no provider keys in the environment.
env -i HOME=$HOME PATH=$PATH /Volumes/C/.hms-tgt/locomo/release/pilot-memsrv --no-auth --bind 127.0.0.1:18777 --data-dir /Volumes/C/.hms-tgt/mh2store/data --input-dim 1536 --max-top-k ${MAXK:-200} --max-queue 1000 > /Volumes/A/.hms-lb/locomo-20261114/multihop2/server.log 2>&1 &
echo $! > /Volumes/A/.hms-lb/locomo-20261114/multihop2/server.pid
