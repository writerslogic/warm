#!/bin/bash
# Pinned warm-rust 0.7.5: build and index the adapter, then the same-session full and tpuf runs.
source ~/.cargo/env
export JAVA_HOME=$HOME/jdk-21.0.8+9; export PATH=$PATH:$JAVA_HOME/bin
cd ~/search-benchmark-game; export CORPUS=$PWD/corpus.json
O=~/fair/pin075; mkdir -p $O; L=$O/pin.log; : > $L
E=engines
rm -rf $E/warm-0.7.5-bp; mkdir $E/warm-0.7.5-bp; tar xzf ~/fair/warm-075-adapter.tgz -C $E/warm-0.7.5-bp
{ uname -a; lscpu | grep -E "Model name|^CPU\(s\)"; rustc --version; date -u; } >> $L
echo "=== index $(date -u +%T)" >> $L
(cd $E/warm-0.7.5-bp && make --no-print-directory compile >> $L 2>&1; echo "compile exit $?" >> $L; rm -rf idx index-time.json)
python3 scripts/time_index.py warm-0.7.5-bp >> $L 2>&1
echo "exit $? index" >> $L; cat $E/warm-0.7.5-bp/index-time.json >> $L; echo >> $L; du -sb $E/warm-0.7.5-bp/idx >> $L
ENG="infino-0.11.26 infino-main tantivy-0.27 lucene-10.5.0 lucene-10.5.0-bp iresearch-26.10.3 warm-0.7.5-bp"
echo "=== full $(date -u +%T)" >> $L
rm -rf results results.json results-full.json
make --no-print-directory bench-full ENGINES="$ENG" >> $L 2>&1
echo "exit $? full" >> $L
cp results-full.json $O/results-full.json 2>/dev/null || cp results.json $O/results-full.json
echo "=== tpuf $(date -u +%T)" >> $L
rm -rf results results.json
python3 src/client.py queries-tpuf.txt warm-0.7.5-bp infino-0.11.26 infino-main >> $L 2>&1
echo "exit $? tpuf" >> $L
cp results.json $O/results-tpuf.json
echo "PIN DONE $(date -u +%T)" >> $L
