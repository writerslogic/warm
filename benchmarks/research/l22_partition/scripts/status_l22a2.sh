#!/bin/bash
cd ~/bigann/piso/l22a 2>/dev/null || { echo "STATUS no-dir"; exit 0; }
cat out2/chain.log 2>/dev/null
grep -h -E '^RESULT|Traceback|Error|Killed' out2/*.log 2>/dev/null
[ -f DONE2 ] && echo "STATUS DONE"
