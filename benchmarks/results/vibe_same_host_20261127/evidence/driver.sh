#!/bin/bash
cd ~/vs
declare -A ORD=( [1]="new base roar mlann" [2]="mlann roar base new" [3]="base new mlann roar" )
while pgrep -f "[r]un_algorithm.py" >/dev/null; do sleep 10; done
for b in 1 2 3; do
  for s in ${ORD[$b]}; do
    for ds in yi-128-ip llama-128-ip; do
      if [ "$(cat work/b$b-$s-$ds/exit 2>/dev/null)" = 0 ]; then continue; fi
      ./run_one.sh $b $s $ds
      echo "$(date -u +%FT%TZ) b$b $s $ds exit=$(cat work/b$b-$s-$ds/exit)" >> driver.log
    done
  done
done
echo ALL_DONE >> driver.log
