#!/bin/bash
# usage: run_one.sh <block> <sys: new|base|roar|mlann> <dataset>
set -u
b=$1; s=$2; ds=$3
V=$HOME/vs
export MAMBA_ROOT_PREFIX=$V/mm
export PATH=$V/mm/envs/vibe/bin:$HOME/.cargo/bin:$PATH
for k in OMP MKL OPENBLAS TBB NUMEXPR VECLIB BLIS GOTO NUMBA JULIA RAYON; do export ${k}_NUM_THREADS=1; done
export OMP_DYNAMIC=false MKL_DYNAMIC=false HDF5_USE_FILE_LOCKING=FALSE
w=$V/work/b$b-$s-$ds; rm -rf $w; mkdir -p $w; ln -s $HOME/vibe/data $w/data; cp $V/vibe/logging.conf $V/vibe/run_algorithm.py $w/
case $s in
  new) vd=$V/vibe-new; alg=warm; defs=$vd/vibe/algorithms; pp=$V/pylib-new;;
  base) vd=$V/vibe-base; alg=warm; defs=$vd/vibe/algorithms; pp=$V/pylib-base;;
  roar) vd=$V/vibe-new; alg=roargraph; defs=$V/defs; pp=;;
  mlann) vd=$V/vibe-new; alg=mlann-rf; defs=$V/defs; pp=;;
esac
cd $w
date -u +%FT%TZ > start
PYTHONPATH=$vd:$pp python $vd/run.py --dataset $ds --algorithm $alg --definitions $defs --local --runs 5 > run.log 2>&1
echo $? > exit; date -u +%FT%TZ > end
