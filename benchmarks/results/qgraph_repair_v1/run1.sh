#!/bin/bash
B=./qgraph-repair-bench.pinned
EF=10,16,20,24,32,48,64,96,128,192,256
COMMON="--extra-degree 8 --antihub 16 --entry-points 64 --repair-ef 32 --train-split 5000 --split-seed 165 --ef $EF --repeats 3"
nice -n 15 $B --data $HOME/.cache/hms-bench/glove-100-angular --dim 100 --metric angular --n 1183514 $COMMON --cache ./glove.graph --out ./glove-full.json > ./glove-full.log 2>&1
echo "glove exit=$?" >> ./run.status
nice -n 15 $B --data $HOME/.cache/hms-bench/sift-128-euclidean --dim 128 --metric euclidean --n 1000000 $COMMON --cache ./sift.graph --out ./sift-full.json > ./sift-full.log 2>&1
echo "sift exit=$?" >> ./run.status
