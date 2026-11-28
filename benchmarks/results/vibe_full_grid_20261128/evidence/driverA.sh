#!/bin/bash
cd ~/vs
mkdir -p workA
for sd in "roar yi-128-ip defsA/roargraph" "roar llama-128-ip defsA/roargraph" "mlann yi-128-ip defsA/mlann" "mlann llama-128-ip defsA/mlann" "new yi-128-ip defsA/warm" "new llama-128-ip defsA/warm"; do
  set -- $sd
  [ "$(cat workA/A-$1-$2/exit 2>/dev/null)" = 0 ] && continue
  ./run_cfg.sh A $1 $2 $HOME/vs/defsA 3 3
  echo "$(date -u +%FT%TZ) A $1 $2 exit=$(cat workA/A-$1-$2/exit)" >> driverA.log
done
echo ALL_DONE >> driverA.log
