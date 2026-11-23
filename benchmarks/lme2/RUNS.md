# LongMemEval-V2 run manifest

Every V2 number WARM has recorded, with what produced it. Small tier only. "Zero-LLM memory" means `memory.query` makes no generative model call (retrieval, ranking and packing only); the reader is the benchmark's fixed answering model and is not part of memory. The reference protocol is: Qwen3.5-9B bf16 reader, one pass, temperature 0.6 / top-p 0.95 / top-k 20, thinking on, 20000 completion tokens; gpt-5.2 judge at medium effort for the 156 abstention and gotcha questions, local string or choice match for the rest; all 451 questions per submission.

The run artifacts live in the private workspace `/Volumes/A/.hms-lb/lme2`, which is not a git repository, so the private runs have no commit; their configs are cited by file. Development = the 228 ids not in `held_ids.json`.

| Run | Population | Score | Memory-side generative calls | Reader passes | Judge | Zero-LLM memory | Reference protocol |
|---|---|---:|---|---|---|---|---|
| Round 5 `slice12i` | dev 228 | 48.68 | none | 1 | gpt-5.2 medium | yes | dev only, not a submission |
| Round 4 (same ids) | dev 228 | 45.61 | not recorded here | 1 | gpt-5.2 | unknown | dev only |
| Round 6 `wns_e` | dev 228 / held-out 223 | 57.46 / 51.1 | Qwen3.5-9B: per-trajectory notes and a per-question support analysis | 1 | gpt-5.2 medium | no | dev and held-out, not 451 together |
| BR6-c35 | dev 228 | 63.6 | Qwen3.5-35B-A3B-FP8 controller, 12 tool calls, writes a brief | 1 | gpt-5.2 | no | dev only |
| G1 | dev 228 / held-out 223 / all 451 | 81.6 / 73.5 / 77.6 | Gemini 3.6 Flash on Vertex: controller, VERIFY, CRUX, OPTCHECK, pass-2 controller, ANSWER-CHECK | 1, or 2 when ANSWER-CHECK finds a contradiction | gpt-5.2 medium | no | no: the second reader pass is outside the protocol, and the frozen run used `ARCH_TRUST_CACHE=1` |
| `warm_balanced`, `warm_wide` | all 451, retrieval only | none | none | 0 (no reader output completed) | none | yes | no score |

Details:

- **The 48.7 figure is round 5 `slice12i` on development: 48.68 of 228** (`round5/dev_score.txt`; round 6 `FROZEN_CONFIG.md` cites it as "round 5 slice12i 48.68"). Memory: `warm_memory/run_lme2.py` preset `slice12i`, the top 12 states by hybrid BM25 and WARM-embedder retrieval, up to 320000 characters, screenshots attached for the top 6 hits. Reader: `round5/serve_r5.sh`, vLLM `Qwen/Qwen3.5-9B --dtype bfloat16`, cache `round5/reader_cache_slice12i.json`. Judge: `runs_judged/slice12i_*_subset*/run_args.json` (`evaluator_model` gpt-5.2, effort medium, 4096 tokens). Mean context 111,292 tokens. This is the recorded zero-LLM-memory baseline. It is a development number, so it is not comparable to the held-out 74.9 reference or to a 451-question leaderboard score.
- **Round 6:** `round6/FROZEN_CONFIG.md`. The notes were written by Qwen3.5-9B offline (`round6/notes.json`). The support analysis is a Qwen3.5-9B generation at query time over the top 6 states. Both are generative memory-side calls by the same model as the reader.
- **G1:** `benchmarks/lme2/README.md` and `arch7/SUBMISSION.md`. Its 73.5 is the score of a pipeline with a hosted Gemini controller and checking stages and a conditional second reader pass. It is not the score of a zero-LLM memory workflow and is not a reference-protocol result.
- **Caches:** reader outputs are cached per run (`reader_cache_*.json`, `final-arch/rc_*.json`) and judgments in `judge_cache.jsonl`. No score above was recomputed for this manifest except where `headroom/evidence_audit.py` rescored phrase questions locally.
