# LongMemEval context-assembly experiments (dev only)

Research harness; paths point at the maintainer workspace (`/Volumes/A/.hms-lb/longmemeval`) and the cached dataset. `render.py` builds the reader context (official TEMPLATE unchanged, memory notes added inside the history slot), `run_arm.py` scores an arm with the official judge script, `prep_extract.py` + `extract_modal.py` run the ingest extraction with Qwen3-30B-A3B-Instruct-2507 on Modal, `breakdown.py` is the zero-API error breakdown.

Results and caveats: `docs/benchmarks/LONGMEMEVAL-CONTEXT-ASSEMBLY.md`, `benchmarks/results/longmemeval_context_assembly_dev_v1.json`.
