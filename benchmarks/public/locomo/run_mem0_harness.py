# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Run Mem0's own LoCoMo/LongMemEval/BEAM harness with WARM behind Mem0Client.

usage: python -I run_mem0_harness.py --harness DIR --benchmark locomo -- <args of benchmarks.<name>.run>
DIR is a checkout of mem0ai/memory-benchmarks (read, never modified). The harness code runs as published;
the only substitution is Mem0Client -> WarmClient. Set OPENAI_BASE_URL and WARM_EMBED_BASE_URL to the
stub (stub_llm.py) for a free plumbing run.
"""
import argparse, asyncio, importlib, os, sys
os.environ.pop('OPENROUTER_API_KEY', None)

ap = argparse.ArgumentParser()
ap.add_argument('--harness', required=True)
ap.add_argument('--benchmark', default='locomo', choices=('locomo', 'longmemeval', 'beam'))
args, rest = ap.parse_known_args()
sys.path.insert(0, args.harness)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import benchmarks.common.mem0_client as mc
from mem0_harness_warm import WarmClient
mc.Mem0Client = WarmClient
mod = importlib.import_module(f'benchmarks.{args.benchmark}.run')
mod.Mem0Client = WarmClient
sys.argv = [f'benchmarks.{args.benchmark}.run'] + [a for a in rest if a != '--']
r = mod.main()
if asyncio.iscoroutine(r):
    asyncio.run(r)
