#!/bin/bash
# Phase 1: unpack both trees, build both adapters, index both (timed by the harness).
source ~/.cargo/env
export JAVA_HOME=$HOME/jdk-21.0.8+9; export PATH=$PATH:$JAVA_HOME/bin
L=~/fair/out/final1.log; : > $L
cd ~/fair; rm -rf warm-new warm-main; mkdir warm-new warm-main
tar xzf warm-new.tgz -C warm-new; tar xzf warm-main.tgz -C warm-main
cd ~/search-benchmark-game; export CORPUS=$PWD/corpus.json
E=engines
for n in new main; do
  rm -rf $E/warm-$n-final; mkdir $E/warm-$n-final
  for f in src Cargo.toml Cargo.lock Makefile details.json README.md; do cp -r $E/warm-fair-bp/$f $E/warm-$n-final/; done
  sed -i "s#~/fair/warm-rust#~/fair/warm-$n#" $E/warm-$n-final/Cargo.toml
  sed -i 's/ --locked//' $E/warm-$n-final/Makefile
done
for n in new main; do
  e=warm-$n-final
  echo "=== index $e $(date -u +%T)" >> $L
  (cd $E/$e && make --no-print-directory compile >> $L 2>&1; echo "compile exit $?" >> $L; rm -rf idx index-time.json)
  python3 scripts/time_index.py $e >> $L 2>&1
  echo "exit $? $e" >> $L
  cat $E/$e/index-time.json >> $L; echo >> $L
  du -sb $E/$e/idx >> $L
done
# new build over the main-built (old-format) index: the non-table path
rm -rf $E/warm-new-oldidx; mkdir -p $E/warm-new-oldidx/target/release
cp $E/warm-new-final/details.json $E/warm-new-oldidx/
cp $E/warm-new-final/target/release/build_index $E/warm-new-final/target/release/do_query $E/warm-new-oldidx/target/release/
cat > $E/warm-new-oldidx/Makefile <<'MK'
clean:
	rm -f idx

compile:
	@true

index: idx

serve:
	@../warm-new-final/target/release/do_query idx
MK
ln -sfn ../warm-main-final/idx $E/warm-new-oldidx/idx
cp $E/warm-main-final/index-time.json $E/warm-new-oldidx/index-time.json
echo "FINAL1 DONE $(date -u +%T)" >> $L
