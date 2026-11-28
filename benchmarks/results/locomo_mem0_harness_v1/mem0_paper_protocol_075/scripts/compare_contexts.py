"""Compare the reader prompts (contexts) of two retrieval-only LoCoMo runs, byte for byte.
Usage: uv run --no-project python -I compare_contexts.py OLD/predictions.jsonl NEW/predictions.jsonl"""
import json, sys

def load(path):
    with open(path) as f:
        return {(r['sample_id'], r['qa_index']): r for r in map(json.loads, f)}

old, new = load(sys.argv[1]), load(sys.argv[2])
assert old.keys() == new.keys(), f'question sets differ: {len(old)} vs {len(new)}'
diff = [k for k in sorted(old) if old[k]['prompt'] != new[k]['prompt']]
assert all((old[k]['prompt_sha256'] == new[k]['prompt_sha256']) == (k not in set(diff)) for k in old)
print(f'questions {len(old)}, categories {sorted({r["category"] for r in old.values()})}, identical {len(old) - len(diff)}, different {len(diff)}')
by_cat = {}
for k in diff:
    by_cat[old[k]['category']] = by_cat.get(old[k]['category'], 0) + 1
print('different by category:', by_cat)
for k in diff[:5]:
    a, b = old[k], new[k]
    print(k, 'ids only old:', sorted(set(a['retrieved_a'] + a['retrieved_b']) - set(b['retrieved_a'] + b['retrieved_b']))[:5],
          'only new:', sorted(set(b['retrieved_a'] + b['retrieved_b']) - set(a['retrieved_a'] + a['retrieved_b']))[:5])
sys.exit(1 if diff else 0)
