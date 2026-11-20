#!/bin/bash
source ~/vllm_env/bin/activate
export VLLM_USE_FLASHINFER_SAMPLER=0
pkill -f "vllm serve"; sleep 2
nohup vllm serve Qwen/Qwen3.5-9B --port 8023 --dtype bfloat16 --max-model-len 150000 --gpu-memory-utilization 0.92 \
  --max-num-batched-tokens 16384 --max-num-seqs 16 --reasoning-parser qwen3 --served-model-name Qwen/Qwen3.5-9B \
  --limit-mm-per-prompt '{"image":8}' > ~/vllm.log 2>&1 &
