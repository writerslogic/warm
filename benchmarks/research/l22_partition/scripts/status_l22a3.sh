#!/bin/bash
cd ~/bigann/piso/l22a 2>/dev/null || { echo "STATUS no-dir"; exit 0; }
cat out3/chain.log 2>/dev/null
grep -h -E '^RESULT|Traceback|Error|Killed|AssertionError' out3/a3.log 2>/dev/null
[ -f DONE3 ] && echo "STATUS DONE"
