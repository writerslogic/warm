#!/bin/bash
cd ~/w/kill/round2
export OMP_NUM_THREADS=3 OPENBLAS_NUM_THREADS=3
PY=~/v/bin/python
O=~/w/out2
$PY k5a.py oracle --R 12 --N 64 --seeds 2 --nq 8 --bprs 32,64,128 --out $O/o_R12_N64.json > $O/o_R12_N64.log 2>&1
$PY k5a.py acc --R 12 --u 3 --N 4  --seeds 2 --nq 16 --nq-tune 8 --bprs 16384,65536 --out $O/p_R12_u3_N4.json > $O/p_R12_u3_N4.log 2>&1
$PY k5a.py acc --R 12 --u 3 --N 16 --seeds 2 --nq 16 --nq-tune 8 --bprs 8192,32768 --out $O/p_R12_u3_N16.json > $O/p_R12_u3_N16.log 2>&1
$PY k5a.py oracle --R 12 --N 256 --seeds 1 --nq 6 --bprs 32,64,96 --out $O/o_R12_N256.json > $O/o_R12_N256.log 2>&1
export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
$PY k5a.py time --tuned $O/a_R12_u3_N256.json --ntime 5 --out $O/t_a_R12_u3_N256.json > $O/t_a_R12_u3_N256.log 2>&1
echo done > $O/A_DONE
