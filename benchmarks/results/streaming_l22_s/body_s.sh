#!/bin/bash
source ~/.cargo/env
TAG=$1; CFG=$2; B=~/bigann/l22bench; R=$B/runS/$TAG; I=$B/run10/in; H=$B/heldout; LOG=$B/chainS.log
W="/mnt/c/Users/David Condrey"
mkdir -p $R
NEW=$(sha256sum "$W/l22srcS.tgz" | cut -d" " -f1)
if [ "$NEW" != "$(cat $B/srcS.sha 2>/dev/null)" ]; then
  cd $B/l22 && rm -rf src && tar xzf "$W/l22srcS.tgz" || exit 1
  echo $NEW > $B/srcS.sha
  env CARGO_TARGET_DIR=$B/targetS nice -n 10 cargo test --release --lib --features experimental partition > $B/runS/build.log 2>&1 || { echo BUILDFAIL >> $LOG; exit 1; }
fi
cd $B/l22
cp "$W/$CFG" $R/cfg.txt; sed -i 's/\r$//' $R/cfg.txt
while IFS="|" read -r label op envs; do
  [ -z "$label" ] && continue
  envs=${envs//@I@/$I}; b=$I/base_op$op.fbin; n=$(od -An -tu4 -N4 $b | tr -d " ")
  if [ "${QSET:-held}" = board ]; then Q=$B/run8/queries10k.bin; G=$I/gt_op${op}_500.bin; else Q=$H/heldout_q2000.bin; G=$H/gt_heldout2000_op$op.bin; fi
  echo "== $TAG/$label op=$op start $(date -u +%H:%M:%S) load: $(cut -d" " -f1-3 /proc/loadavg)" >> $LOG
  env RAYON_NUM_THREADS=8 WARM_L22_ACC_BASE=$b WARM_L22_ACC_N=$n WARM_L22_ACC_QUERIES=$Q WARM_L22_ACC_GT=$G WARM_L22_ACC_OUT=$R/${label}_op$op.json \
    $envs \
    taskset -c 0-7 nice -n 19 env CARGO_TARGET_DIR=$B/targetS cargo test --release --lib --features experimental partition_prefix_acceptance -- --ignored --nocapture > $R/${label}_op$op.log 2>&1
  echo "== $TAG/$label op=$op exit $? $(date -u +%H:%M:%S)" >> $LOG
done < $R/cfg.txt
touch $B/DONE_S_$TAG
