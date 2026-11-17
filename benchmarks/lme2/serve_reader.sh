#!/bin/bash
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
# Serve the fixed G1 reader: Qwen3.5-9B, bf16, vLLM 0.33.0 (flags as in the frozen runs).
# MTP speculative decoding is distribution-preserving; it changes throughput, not the sampled distribution.
# usage: serve_reader.sh [port]   (vllm on PATH: uv pip install vllm==0.33.0)
set -euo pipefail
PORT=${1:-8023}
export VLLM_USE_FLASHINFER_SAMPLER=0
exec vllm serve Qwen/Qwen3.5-9B --port "$PORT" --dtype bfloat16 --max-model-len 150000 --gpu-memory-utilization 0.92 \
  --max-num-batched-tokens 16384 --max-num-seqs 32 --reasoning-parser qwen3 --served-model-name Qwen/Qwen3.5-9B \
  --limit-mm-per-prompt '{"image":24}' --speculative-config '{"method":"mtp","num_speculative_tokens":2}'
