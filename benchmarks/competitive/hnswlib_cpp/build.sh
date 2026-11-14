#!/usr/bin/env bash
# Builds the native hnswlib driver against the hnswlib 0.8.0 sdist the Python binding is built from,
# with the flags that sdist's setup.py passes on Linux: -O3 -march=native -fopenmp -std=c++14.
# usage: build.sh <out-dir>   (prints the driver path; headers are cached under <out-dir>)
set -euo pipefail
OUT=${1:?out dir}
VERSION=0.8.0
SDIST_URL=https://files.pythonhosted.org/packages/cf/7a/1a9b1405f2eb59515f06c3074750b03e0e96edf7fee0f6dd6df81d9c21d7/hnswlib-$VERSION.tar.gz
SDIST_SHA256=cb6d037eedebb34a7134e7dc78966441dfd04c9cf5ee93911be911ced951c44c
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
if [ ! -f "$OUT/hnswlib-$VERSION/hnswlib/hnswalg.h" ]; then
  curl -fsSL -o "$OUT/hnswlib.tar.gz" "$SDIST_URL"
  echo "$SDIST_SHA256  $OUT/hnswlib.tar.gz" | sha256sum -c - >&2
  tar xzf "$OUT/hnswlib.tar.gz" -C "$OUT"
fi
CXX=${CXX:-g++}
"$CXX" -O3 -march=native -fopenmp -std=c++14 -pthread -I"$OUT/hnswlib-$VERSION" "$HERE/driver.cpp" -o "$OUT/hnswlib-driver"
echo "$OUT/hnswlib-driver"
