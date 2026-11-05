# K-mer encoder evaluation (2026-11-03)

Question (issue 4): does a k-mer sparse encoding of DNA reads show high noise tolerance for
sequence matching?

**Verdict: not demonstrated beyond what exact k-mer overlap already gives.** The encoder tracks
exact k-mer containment and never exceeds it by more than noise (largest margin of SDR
containment over exact containment across all k, dimensions and conditions: +0.004 recall@1, k=9,
20% substitution). At high substitution it falls below it, and
a 2,048-hash MinHash sketch at 16 KB per reference beats it at k=15. The data are synthetic random
sequence only.

Result file: `benchmarks/results/kmer_encoder_eval_v1.json` (every row, with Wilson 95% intervals
and hit counts). Harness: `src/bin/research/kmer-eval.rs`, produced by code revision `8a9c76b9`.
Encoder: `src/core/experimental/kmer.rs`. User guide: [K-mer encoder](../guides/kmer-encoder.md).

## Method

**Encoder.** Each window of `k` bases is hashed (after canonicalization to the smaller of the
k-mer and its reverse complement) to 2 active indices in `[0, dim)`. The sequence vector is the
union of its k-mer vectors, an `EntangledHVec`, scored with the engine's own
`containment_similarity` (fraction of the read's active indices present in the reference) and
`similarity` (Jaccard). This is a Bloom filter over the k-mer set. Majority-vote
`EntangledHVec::bundle` is unusable here: bundling the 4,986 k-mer vectors of one 5 kb reference
(k=15, dim 65,536) kept 0 active indices, against 9,246 for the union
(`majority_bundle_check` in the result file).

**Data.** Seed 20261103. 1,000 references of 5,000 bp, i.i.d. uniform ACGT. 500 reads per
condition of 150 bp, source reference and start uniform, each reverse-complemented with
probability 0.5. Noise is per-base and independent: substitution to one of the other three bases at
0, 1, 5, 10, 15, 20%; indels (a deletion or a random-base insertion with equal probability) at 1 and
5% per base with no substitution (mean read length 149.97 and 149.84).

**Methods**, all using the encoder's own k-mer extraction so alphabet, strand and ambiguity rules
are shared:

- `exact_jaccard`, `exact_containment`: canonical distinct k-mer sets, 8 bytes per k-mer.
- `minhash_bottom_s_jaccard`: Mash estimator, s=256 (2 KB per reference) and s=2048 (16 KB).
- `minhash_scaled_containment`: keep hashes below 2^64/8 (about 5 KB per reference), containment
  of the read sketch in the reference sketch.
- `sdr_canonical_dim{65536,262144,1048576}`: the encoder at 2 bits per k-mer, 4 bytes per active
  index. A non-canonical encoder is included at k=15, dim 1,048,576.

**Metric.** recall@1 and recall@10 of the source reference among all 1,000. Rank is
`1 + #(other references scoring >= the source)`: ties count against the method. Chance is 0.001
and 0.01. Reads are shared across methods inside a condition, so differences between methods are
paired; the 95% intervals in the file are marginal and span up to about 0.09 at recall 0.5.

## Results

recall@1, k=15 (500 reads per cell; bytes per reference in parentheses):

| Method | 0% | 1% | 5% | 10% | 15% | 20% | indel 1% | indel 5% |
|---|---|---|---|---|---|---|---|---|
| exact containment (39.9 KB) | 1 | 1 | 1 | 0.990 | 0.874 | 0.582 | 1 | 1 |
| SDR dim 2^20, containment (39.7 KB) | 1 | 1 | 1 | 0.978 | 0.776 | 0.444 | 1 | 1 |
| SDR dim 2^18, containment (39.1 KB) | 1 | 1 | 1 | 0.948 | 0.664 | 0.320 | 1 | 1 |
| SDR dim 2^16, containment (37.0 KB) | 1 | 1 | 0.998 | 0.844 | 0.410 | 0.154 | 1 | 1 |
| MinHash s=2048 (16.4 KB) | 1 | 1 | 1 | 0.970 | 0.780 | 0.512 | 1 | 1 |
| MinHash s=256 (2.0 KB) | 0.998 | 1 | 0.926 | 0.676 | 0.370 | 0.186 | 1 | 0.922 |
| scaled MinHash 1/8 (5.0 KB) | 1 | 1 | 0.990 | 0.868 | 0.582 | 0.326 | 1 | 0.996 |
| SDR dim 2^20, not canonical | 0.500 | 0.482 | 0.460 | 0.504 | 0.404 | 0.198 | 0.504 | 0.506 |

recall@1 at 20% substitution and at 5% indels, by k (SDR is dim 2^20; the file has all methods and
dimensions):

| k | exact 20% | SDR 20% | MinHash 2048 20% | exact indel 5% | SDR indel 5% |
|---|---|---|---|---|---|
| 9 | 0.656 | 0.660 | 0.446 | 1 | 1 |
| 11 | 0.716 | 0.694 | 0.530 | 1 | 1 |
| 15 | 0.582 | 0.444 | 0.512 | 1 | 1 |
| 21 | 0.250 | 0.158 | 0.180 | 0.998 | 0.992 |

recall@10 at k=15, 20% substitution: exact 0.650, SDR 0.516, MinHash 2048 0.568, MinHash 256 0.192.

## What the numbers support

Measured:

- The SDR is a faithful but lossy copy of exact k-mer containment. Its loss grows as `dim` shrinks
  (denser vector, higher false-positive floor) and as the noise floor approaches the few intact
  k-mers left in a read: about 5 expected intact 15-mers of 136 at 20% substitution.
- Canonical k-mers are required for mixed-strand reads: without them recall is about 0.5 at every
  noise level up to 10%.
- Recall at high noise depends strongly on `k`. For exact matching and the SDR, k=9 and 11 hold
  more at 15 to 20% substitution than k=15 or 21. The MinHash rows do not follow that order
  (s=256 at 20%: 0.040 at k=9, 0.186 at k=15), because a sketch keeps few k-mers either way.
- Indels at 5% per base cost nothing for exact matching and the SDR at k=9 to 15 (recall 1), and
  cost 0.992 for the SDR at k=21.
- The SDR is not smaller than the exact set at the dimensions where it matches it (39.7 KB against
  39.9 KB per reference). Smaller dimensions lose recall. MinHash s=2048 uses 16 KB.

Inference, not tested: the Bloom-filter structure means SDR recall is bounded above by exact
containment at the same `k`; the data are consistent with that bound.

## Not tested, and limits

- No real genome. References are i.i.d. uniform sequence: no repeats, no low-complexity regions,
  no GC bias, no homologs. Real genomes have all of these, and each changes false-positive rates.
- One reference length (5,000 bp), one read length (150 bp), 1,000 references, `k` in 9, 11, 15,
  21, 2 bits per k-mer. Nothing outside that range is claimed; do not extrapolate to chromosome
  scale or millions of references.
- Substitution and indel noise are independent per base. Sequencing error profiles (homopolymer
  indels, position-dependent error) were not modelled. IUPAC and `N` handling is covered by unit
  tests, not by this evaluation.
- MinHash baselines are the two standard estimators at the sizes listed; they were not tuned. The
  asymmetry between a 150 bp read and a 5 kb reference hurts the Jaccard estimator at small `s`.
- Throughput and latency were not measured. The harness binary was built with LTO off and
  opt-level 1.

## Reproduce

```sh
CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=64 CARGO_PROFILE_RELEASE_OPT_LEVEL=1 \
  cargo build --release --features experimental --bin kmer-eval
target/release/kmer-eval --code-rev <rev> --out benchmarks/results/kmer_encoder_eval_v1.json
```

The default arguments are the configuration reported here. All randomness derives from the seed;
a second run to confirm identical output was not made. The 477-second run was on a shared machine.
