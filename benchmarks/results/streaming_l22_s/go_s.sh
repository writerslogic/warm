#!/bin/bash
W="/mnt/c/Users/David Condrey"
cp "$W/pre_s.sh" ~/bigann/l22bench/pre_s.sh && sed -i 's/\r$//' ~/bigann/l22bench/pre_s.sh || exit 1
rm -f ~/bigann/l22bench/DONE_S_$1; timeout 10800 bash ~/bigann/l22bench/pre_s.sh $1 $2; echo "exit $?"
