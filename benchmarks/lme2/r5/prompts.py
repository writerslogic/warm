#!/usr/bin/env python3
# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Prompt stage of the upstream LongMemEval-V2 harness for the R5 baseline and its matched variants.

Builds memory and queries for one domain and writes <out>/prompt_rows.jsonl, stopping before the
reader. The harness arguments are those of the R5 run (round5/runs/slice12i_*/run_args.json).

    uv run --python /Volumes/A/.hms-lb/lme2/venv/bin/python benchmarks/lme2/r5/prompts.py \
        --system slice12i --domain web --out <dir>

systems: slice12i (R5, warm_slice), slice12i_r5check (warm_bound with render r5; must equal
slice12i), slice12i_bound (condition C), slice12i_<candidate> (see CANDIDATES).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
LME2 = Path(os.environ.get("LME2_ROOT", "/Volumes/A/.hms-lb/lme2"))
sys.path.insert(0, str(LME2 / "upstream"))
sys.path.insert(0, str(HERE))

import memory_modules  # noqa: E402,F401
import warm_hybrid  # noqa: E402,F401
import warm_slice  # noqa: E402,F401
import warm_bound  # noqa: E402,F401
import warm_select  # noqa: E402,F401
from data.public_data import materialize_runtime_haystack, materialize_runtime_questions, write_json  # noqa: E402

R5 = {"top_states": 12, "max_context_chars": 320000, "screens_root": str(LME2 / "data" / "screenshots"), "image_hits": 6}
SYSTEMS = {
    "slice12i": ("warm_slice", R5),
    "slice12i_r5check": ("warm_bound", {**R5, "render": "r5"}),
    "slice12i_bound": ("warm_bound", {**R5, "render": "bound"}),
    "slice12i_pagecover": ("warm_select", {**R5, "render": "r5", "select": "pagecover"}),
    "slice12i_pagecover_bound": ("warm_select", {**R5, "render": "bound", "select": "pagecover"}),
}
HARNESS = [
    "--model", "Qwen/Qwen3.5-9B", "--api-key-env", "OPENAI_API_KEY",
    "--temperature", "0.6", "--top-p", "0.95", "--top-k", "20",
    "--max-completion-tokens", "20000", "--memory-context-max-tokens", "200000",
    "--reader-max-concurrent-requests", "16", "--prompt-build-max-workers", "1",
    "--evaluator-model", "gpt-5.2", "--evaluator-api-key-env", "OPENAI_API_KEY",
    "--evaluator-reasoning-effort", "medium", "--evaluator-max-completion-tokens", "4096",
]


def build(memory_type: str, params: dict, domain: str, out: Path, embed_cache: Path, question_ids=None) -> Path:
    """Runs the harness prompt stage and returns the prompt_rows.jsonl it wrote."""
    import evaluation.harness as H

    data_root = LME2 / "data"
    held = set(json.load(open(HERE.parent / "held_ids.json")))
    if question_ids is None:
        # Development ids of this domain only: no held-out question reaches memory, a row or a
        # dispatch file. Each row depends only on its own question and the shared haystack, so
        # this does not change any development row.
        question_ids = sorted(q["id"] for q in map(json.loads, open(data_root / "questions.jsonl"))
                              if q["domain"] == domain and q["id"] not in held)
    if held & set(question_ids):
        raise SystemExit("held-out ids requested")
    rt = out / "runtime_inputs"
    rt.mkdir(parents=True, exist_ok=True)
    questions = materialize_runtime_questions(data_root=data_root, domain=domain, question_ids=question_ids, limit=None, output_path=rt / "questions.json")
    materialize_runtime_haystack(data_root=data_root, tier="small", selected_questions=questions, output_path=rt / "haystack.json")
    write_json(rt / "memory_config.json", {"memory_type": memory_type, "memory_params": {"cache_dir": str(embed_cache), **params}})

    async def stop(a, rows):
        raise SystemExit(0)

    H.generate_all_reader_outputs = stop
    argv = ["evaluation.harness", "--domain", domain,
            "--questions-path", str(rt / "questions.json"), "--haystack-path", str(rt / "haystack.json"),
            "--trajectories-path", str(data_root / "trajectories.jsonl"),
            "--memory-config-path", str(rt / "memory_config.json"), "--output-dir", str(out),
            "--base-url", "http://localhost:8023/v1", *HARNESS]
    old = sys.argv
    sys.argv = argv
    try:
        H.main()
    except SystemExit as e:
        if e.code not in (0, None):
            raise
    finally:
        sys.argv = old
    return out / "prompt_rows.jsonl"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--system", required=True, choices=sorted(SYSTEMS))
    ap.add_argument("--domain", required=True, choices=["web", "enterprise"])
    ap.add_argument("--out", required=True)
    ap.add_argument("--embed-cache", default=str(LME2 / "embed_cache"))
    a = ap.parse_args()
    mt, params = SYSTEMS[a.system]
    sys.stdout.write(f"{build(mt, params, a.domain, Path(a.out).resolve(), Path(a.embed_cache))}\n")


if __name__ == "__main__":
    main()
