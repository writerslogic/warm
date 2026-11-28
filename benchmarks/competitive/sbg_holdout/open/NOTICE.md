# Open validation files

- `nq_train_validation.jsonl`: derived from the NQ-open training questions (Natural Questions, Google), licensed
  CC BY-SA 3.0. Attribution: Kwiatkowski et al., "Natural Questions: a Benchmark for Question Answering Research",
  TACL 2019; NQ-open split by Lee et al., ACL 2019. The derived queries (word sequences of questions, with the
  operators of MANIFEST.md) are shared under the same CC BY-SA 3.0 license. This file only: the rest of the
  repository stays Apache-2.0.
- `long_validation.jsonl`: sentences of Project Gutenberg books in the public domain in the United States
  (ids in `../sources.json`), lowercased and stripped of punctuation.
- `random_A.jsonl`, `length_sweep.jsonl`: generated from the benchmark corpus vocabulary; no third-party text beyond
  words and adjacent-word windows of the benchmark's Wikipedia corpus.
