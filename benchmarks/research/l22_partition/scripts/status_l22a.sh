#!/bin/bash
cd ~/bigann/piso/l22a 2>/dev/null || { echo "no l22a dir"; exit 1; }
echo "load: $(cut -d' ' -f1-3 /proc/loadavg)  DONE: $([ -f DONE ] && echo yes || echo no)"
cat out/chain.log 2>/dev/null
grep -h '^RESULT' out/*.log 2>/dev/null
for f in out/*.log; do [ -f "$f" ] && { echo "-- $f"; tail -n 2 "$f"; }; done
