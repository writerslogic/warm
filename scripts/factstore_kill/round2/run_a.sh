#!/bin/bash
# Lead A driver: registered cells, scaling probes, oracle, timing. Done marker: ~/w/out2/A_DONE
cd ~/w/kill/round2
export OMP_NUM_THREADS=4 OPENBLAS_NUM_THREADS=4
PY=~/v/bin/python
O=~/w/out2
for N in 256 1024; do
  for ru in "12 3" "12 4" "12 6" "16 3" "16 4" "16 8"; do
    set -- $ru
    $PY k5a.py acc --R $1 --u $2 --N $N --seeds 20 --nq 100 --nq-tune 16 --out $O/a_R$1_u$2_N$N.json > $O/a_R$1_u$2_N$N.log 2>&1
  done
done
# scaling probes: where does the resonator reach 0.9 as N grows (R=12)
$PY k5a.py acc --R 12 --u 3 --N 1  --seeds 8 --nq 32 --nq-tune 16 --bprs 1024,2048,4096,8192,16384,32768 --out $O/p_R12_u3_N1.json > $O/p_R12_u3_N1.log 2>&1
$PY k5a.py acc --R 12 --u 3 --N 4  --seeds 8 --nq 32 --nq-tune 16 --bprs 2048,4096,8192,16384,32768 --out $O/p_R12_u3_N4.json > $O/p_R12_u3_N4.log 2>&1
$PY k5a.py acc --R 12 --u 3 --N 16 --seeds 8 --nq 32 --nq-tune 16 --bprs 1024,2048,4096,8192,16384 --out $O/p_R12_u3_N16.json > $O/p_R12_u3_N16.log 2>&1
$PY k5a.py acc --R 12 --u 3 --N 64 --seeds 8 --nq 32 --nq-tune 16 --bprs 512,1024,2048,4096 --out $O/p_R12_u3_N64.json > $O/p_R12_u3_N64.log 2>&1
$PY k5a.py acc --R 12 --u 2 --N 64 --seeds 8 --nq 32 --nq-tune 16 --bprs 64,128,256,512,1024,2048 --out $O/p_R12_u2_N64.json > $O/p_R12_u2_N64.log 2>&1
# exhaustive-decode oracle (same bundle, all 256^3 tuples)
$PY k5a.py oracle --R 12 --N 256 --seeds 3 --nq 12 --bprs 16,24,32,48,64,96 --out $O/o_R12_N256.json > $O/o_R12_N256.log 2>&1
$PY k5a.py oracle --R 12 --N 64 --seeds 3 --nq 12 --bprs 16,32,64,128 --out $O/o_R12_N64.json > $O/o_R12_N64.log 2>&1
# single-thread timing at the registered top point and at the probe points
export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
for f in a_R12_u3_N256 a_R12_u3_N1024 a_R12_u6_N256 a_R16_u4_N256 p_R12_u3_N16 p_R12_u3_N64; do
  $PY k5a.py time --tuned $O/$f.json --ntime 6 --out $O/t_$f.json > $O/t_$f.log 2>&1
done
echo done > $O/A_DONE
