#!/bin/bash
# usage: run_cfg.sh <tag> <sys: new|base|roar|mlann> <dataset> <defsdir> <runs> <parallelism> [extra run.py args]
set -u
tag=$1; s=$2; ds=$3; defs=$4; runs=$5; par=$6; shift 6
V=$HOME/vs
export MAMBA_ROOT_PREFIX=$V/mm
export PATH=$V/mm/envs/vibe/bin:$HOME/.cargo/bin:$PATH
for k in OMP MKL OPENBLAS TBB NUMEXPR VECLIB BLIS GOTO NUMBA JULIA RAYON; do export ${k}_NUM_THREADS=1; done
export OMP_DYNAMIC=false MKL_DYNAMIC=false HDF5_USE_FILE_LOCKING=FALSE
w=$V/workA/$tag-$s-$ds; rm -rf $w; mkdir -p $w; ln -s $HOME/vibe/data $w/data; cp $V/vibe/logging.conf $V/vibe/run_algorithm.py $w/
case $s in
  new) vd=$V/vibe-new; alg=warm; pp=$V/pylib-new;;
  base) vd=$V/vibe-base; alg=warm; pp=$V/pylib-base;;
  roar) vd=$V/vibe-new; alg=roargraph; pp=;;
  mlann) vd=$V/vibe-new; alg=mlann-rf; pp=;;
esac
cd $w
date -u +%FT%TZ > start
PYTHONPATH=$vd:$pp python $vd/run.py --dataset $ds --algorithm $alg --definitions $defs --local --runs $runs --parallelism $par "$@" > run.log 2>&1
echo $? > exit; date -u +%FT%TZ > end
