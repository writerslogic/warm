#!/bin/bash
B=./qgraph-repair-bench.pinned
L=/Volumes/A/.hms-i165-logs
EF=10,16,20,24,32,48,64,96,128,192,256
COMMON="--extra-degree 8 --antihub 16 --entry-points 64 --repair-ef 32 --train-split 5000 --split-seed 165 --ef $EF --repeats 3"
nice -n 15 $B --data $HOME/.cache/hms-bench/glove-100-angular --dim 100 --metric angular --n 1183514 $COMMON --cache $L/glove.graph --out $L/glove-full.json > $L/glove-full.log 2>&1
echo "glove exit=$?" >> $L/run2.status
nice -n 15 $B --data $HOME/.cache/hms-bench/sift-128-euclidean --dim 128 --metric euclidean --n 1000000 $COMMON --cache $L/sift.graph --out $L/sift-full.json > $L/sift-full.log 2>&1
echo "sift exit=$?" >> $L/run2.status
nice -n 15 $B --data $HOME/.cache/hms-bench/glove-100-angular --dim 100 --metric angular --n 200000 --extra-train-rows 27768 $COMMON --cache $L/glove200k.graph --out $L/glove-200k-dense.json > $L/glove-200k-dense.log 2>&1
echo "glove200k exit=$?" >> $L/run2.status
