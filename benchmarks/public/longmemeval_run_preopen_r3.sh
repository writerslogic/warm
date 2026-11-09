#!/bin/bash
# chain preopen: gpt-5-mini k30 + opening-assistant-turn predecessor only (structural rule), two passes
set -u; cd /Volumes/A/.hms-lb/longmemeval; D=$HOME/.cache/warm-bench/downloads
export LME_STATE=/Volumes/A/.hms-lb/longmemeval/l01/state-gpt-5-mini-preopen; mkdir -p "$LME_STATE/calls"
for r in 2; do tag="l01-gpt-5-mini-k30-preopen-r$((r+1))"
  OPENAI_BASE_URL=http://127.0.0.1:8787/v1 OPENAI_API_KEY=proxy uv run --no-project --python 3.11 qa_l01.py --allow-nonfrozen --reader gpt-5-mini --topk 30 --pre 1 --pre-mode opening --repeat $r --rankings out/lme500.rankings.json --judges gpt-4o-mini --tag "$tag" --out-dir l01/out --workers 8 > "l01/out/$tag.qa.log" 2>&1; echo "$tag qa exit=$?"
  H="l01/out/$tag.hypotheses.jsonl"; [ -f "$H" ] && OPENAI_BASE_URL=http://127.0.0.1:8787/v1 OPENAI_API_KEY=proxy uv run --no-project --python 3.11 --with openai --with backoff --with numpy --with tqdm python "$D/longmemeval_evaluate_qa_9e0b455f4ef0.py" gpt-4o-mini "$H" "$D/longmemeval_s_cleaned.json" > "l01/out/$tag.judge.log" 2>&1; echo "$tag judge exit=$?"
done; echo PREOPEN_R3_DONE
