#!/bin/bash
W="/mnt/c/Users/David Condrey"
for f in pre_s2.sh body_s.sh; do cp "$W/$f" ~/bigann/l22bench/$f && sed -i 's/\r$//' ~/bigann/l22bench/$f || exit 1; done
rm -f ~/bigann/l22bench/DONE_S_$1; timeout 14400 bash ~/bigann/l22bench/pre_s2.sh $1 $2; echo "exit $?"
