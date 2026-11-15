# Fact store: kill tests for a task where superposition beats the strongest equal-bits baseline

**Outcome (2026-11-14): no candidate survives.** Every task where the vector-symbolic store was a plausible winner either reduces algebraically to a sketch that a conventional structure already is, or loses to a competent baseline at equal bits. This page records the pre-registration, the baselines, the measurements and the single redesign that was not built. Nothing here changes a public claim; it bounds them.

Pre-registration: [`benchmarks/results/factstore_kill_20261114/PREREG.md`](../benchmarks/results/factstore_kill_20261114/PREREG.md) (sha256 `fccea3bfe6948428d2f33878c9e60957805fdbb2e3eb3d3dbd2a81b7bba2e754`, written before any measurement; the K4b/K4c baselines and some reduced sweeps were added after the first look, see Deviations). Harness: `scripts/factstore_kill/` (numpy, one file per task). Raw results: `benchmarks/results/factstore_kill_20261114/*.json`. Reproduce: `python3 scripts/factstore_kill/k1_membership.py --source synth --out k1.json`, `python3 scripts/factstore_kill/k234.py k4c --out k4c.json`, `python3 scripts/factstore_kill/summarize.py <dir>`.

## Kill rule

A candidate survives only if the paired difference (VSA minus the strongest baseline, each at its tuned setting) has a 95% bootstrap CI above +0.01 AUC (K1), +0.02 accuracy (K2, K4) or a more than 10% lower RMSE (K3), in at least 3 cells at 2 or more noise levels. Tuning (VSA counter width and clip, baseline parameters) used separate seeds from reporting. A tie is reported as a tie.

## Outcome by candidate

| Task | Strongest baseline at equal bits | Outcome | Basis |
|---|---|---|---|
| (i) merge without raw facts | Bloom OR, count-min add, IBLT add | tie by construction | The counting bundle is a counting Bloom filter and the dense bundle is a count-sketch; both merge by the same addition. No compute. |
| (iv) O(1) update and delete | counting Bloom, cuckoo filter | dominated | Counting bundle is counting Bloom (needs 32-bit counters to match plain Bloom in `bundle_k_from_n_20261114.json`); cuckoo filters delete at about 12 to 20 bits per item. No compute. |
| (v) privacy or deniability | Gaussian-noised count-min sketch | tie by construction | Noised bundle is a noised linear sketch. No compute. |
| (ii) near-key membership (K1) | stored 1-bit signatures with scan; distance-sensitive Bloom (LSH tokens, parameters tuned) | **dead** (AUC edge of +0.07 to +0.10 on real text in 3 cells where baselines are near chance; TPR at 1% FPR lower; not claimed) | See K1. |
| (ii) near-key lookup (K2) | stored signature plus 10-bit id; LSH static functions | **dead or tie** | See K2. |
| (vi) set similarity (K3) | b-bit MinHash | **dead** | VSA RMSE more than 10% lower in 2 of 30 cells, both inside the selection bias that favours VSA (13 tuned configurations against 4). |
| (iii) partial-cue role-filler retrieval (K4) | exact table scan; parity-address tables | **dead** | See K4. |
| (vi-b) noisy-token set similarity | SimHash of the mean embedding | tie by construction | Random projection is linear, so bundle of signatures equals SimHash of the sum. |

## What the structure is

`bundle_bloom` is a Bloom filter. The counting bundle is a counting Bloom filter. A dense bipolar bundle is a count-sketch. `BlockCodeBundle` is a partitioned counting Bloom filter. Binding with cleanup is the part with no direct sketch analogue, so every measured task is one where binding and cleanup is used.

## K1 near-key membership

Question: "have I stored a fact like this one?" with a noisy query. The dense VSA arm stores the sum of sign-projection codes. It is limited by the key dimension, not by D: crosstalk between two stored keys is set by the correlation of their projected codes, which does not shrink as D grows (the AUC at N=100 is 0.84 at both 16,384 and 65,536 bits). Stored signatures with a max-agreement scan reach AUC at or above 0.99 from about 65 bits per fact, and the distance-sensitive Bloom filter is the better of the two sketches when bits per fact are scarce. A VSA arm that expands the key into sparse band tokens is the distance-sensitive Bloom filter, so that variant is a tie by construction.

Result: the dense VSA arm is below the strongest baseline in nearly every cell, by 0.03 to 0.43 AUC. The kill rule's per-cell test (CI lower bound above +0.01 AUC) is passed in four cells, all in the near-chance regime: synthetic N=3,000, B=2^14, cosine 0.7 (AUC 0.551 against 0.521); LoCoMo N=100, B=2^14, 30% substitution (0.614 against 0.541); LoCoMo N=300, B=2^14, 20% (0.649 against 0.550); LoCoMo N=300, B=2^16, 30% (0.601 against 0.498). In every one of them the baselines are at or near 0.5 and TPR at 1% FPR is lower for VSA (by 0.003 to 0.044). The rule needs 3 cells at 2 noise levels on both synthetic and real keys; synthetic has one cell, so it is not met. The real-text edge is probably the sum statistic avoiding the max-over-N penalty of a signature scan when the per-item signal is near zero; it is a detector with AUC 0.6 to 0.65 and no usable operating point, so it is recorded, not claimed. LoCoMo ran N=100 and N=300 only (N=1,000 did not finish in the time box). Streaming saturating counters (online insert) are lower still.

## K2 near-key lookup

Return a 10-bit value id for a noisy key (N 300 to 3,000, B 2^14 and 2^16). VSA (bind sign code to a value atom, cleanup over 1,024 atoms) ties stored signature plus id only where bits per fact exceed about 200 (N=300, B=2^16, cosine 0.9 and above: 1.000 against 1.000) and is below it elsewhere (for example 0.82 against 0.99 at N=1,000, B=2^16, cosine 0.9). LSH static functions are far below both because one noisy key lands in the wrong bucket in most tables.

## K4 partial-cue role-filler retrieval

R roles, 256 fillers per role, one or two roles missing from the cue. The strongest conventional structure is not the table but parity-address tables: a static function keyed by two fields stores the sum of all fields mod 257, so any single missing field is recovered from the others at about 20 bits per record regardless of R (two disjoint address pairs cover every missing role); two missing fields need three tables with two parity symbols each, about 59 bits. This beats the superposed record store at every tested budget below 48 bits per record for one missing role (0.998 against 0.82 at 24 bits, R=6; 1.000 against 0.86 at 24 bits, R=12), and for two missing roles at R=12 at every tested budget (0.975 against 0.875 at 48 bits). For two missing roles at R=6 the exact table (48 bits) or its subset beats the VSA store at every budget from 6 to 60 bits (for example 1.00 against 0.90 at 48 bits). The VSA store ties only once bits per record exceed the exact table (60 and 96 bits, one missing role). Where the VSA store alone beats the naive baseline (an exact table that keeps a fraction of the records), it does so against a baseline that is not the strongest. With a corrupted cue the VSA store returns chance (0.005) while a tolerant scan of the table returns 1.0, because product binding has no tolerance for a wrong filler.

## Tested range and where each result stops

See the generated tables below. K1: N 100 to 10,000 (synthetic), 100 and 300 (LoCoMo), B from 2^14 to 2^18 bits, noise cosine 0.7 to 0.95 (synthetic) and 10 to 30% character substitution (LoCoMo sentences, hashed character trigrams, centred). K4: N 200 to 1,600, R 6 and 12, V 256.

## Deviations from the pre-registration

- K4b (single missing role, baseline "exact table keeping a fraction of records" and "Bloom of triples") was run before the parity-address baseline was added in K4c; K4b is kept for the record and is not the strongest comparison.
- Some cells ran with fewer seeds than registered: K1 synthetic N=3,000 and 10,000, K1 LoCoMo, K2 and K4. The seed count is in each results file.
- Real-data K1 first ran on uncentred tf-idf vectors, where the bundle score was anticorrelated with membership (AUC below 0.5); centring was added to the shared encoder for all arms and the run repeated. The uncentred file is kept as `k1_locomo_uncentred.json`.
- The K3 tuning used the same pairs as the report, which favours both arms and favours VSA more (more configurations).

## Most promising redesign not built

A resonator-decoded record store for three or more unknown roles at R of 12 or more. Parity-address tables need (u+1) disjoint address sets and u symbols each, so cost grows as u(u+1) (about 118 bits at u=3), and the exact table costs 8R bits (96 at R=12). The measured superposed store needs a roughly constant 48 to 60 bits per record for u=2 and its requirement grows with u times log2 of the filler count, so it may cross below both baselines at u of 3 or more. That needs a resonator network for the joint decode (256^3 candidates is not enumerable) and a query-time comparison, since the table scan is orders of magnitude faster per query. It was not built.

## Generated tables

### K1 near-key membership, synth keys (AUC; VSA = best of w, kappa tuned on tuning seeds)

| N | B | level | strongest baseline | baseline AUC | VSA AUC | VSA - baseline [95% CI] | VSA TPR@1% - baseline [CI] | win cell |
|---|---|---|---|---|---|---|---|---|
| 100 | 16384 | 0.95 | sig | 1.000 | 0.838 | -0.162 [-0.170, -0.155] | -0.817 [-0.840, -0.792] | - |
| 100 | 16384 | 0.9 | sig | 1.000 | 0.816 | -0.184 [-0.194, -0.174] | -0.853 [-0.873, -0.831] | - |
| 100 | 16384 | 0.8 | sig | 1.000 | 0.775 | -0.225 [-0.236, -0.215] | -0.891 [-0.907, -0.875] | - |
| 100 | 16384 | 0.7 | sig | 0.998 | 0.729 | -0.268 [-0.275, -0.262] | -0.881 [-0.895, -0.868] | - |
| 100 | 65536 | 0.95 | sig | 1.000 | 0.841 | -0.159 [-0.167, -0.152] | -0.820 [-0.844, -0.793] | - |
| 100 | 65536 | 0.9 | sig | 1.000 | 0.819 | -0.181 [-0.191, -0.172] | -0.857 [-0.876, -0.838] | - |
| 100 | 65536 | 0.8 | sig | 1.000 | 0.777 | -0.223 [-0.234, -0.213] | -0.895 [-0.910, -0.879] | - |
| 100 | 65536 | 0.7 | sig | 1.000 | 0.733 | -0.267 [-0.274, -0.261] | -0.920 [-0.932, -0.905] | - |
| 300 | 16384 | 0.95 | sig | 0.999 | 0.714 | -0.285 [-0.290, -0.281] | -0.920 [-0.929, -0.912] | - |
| 300 | 16384 | 0.9 | sig | 0.994 | 0.695 | -0.299 [-0.305, -0.294] | -0.848 [-0.863, -0.831] | - |
| 300 | 16384 | 0.8 | sig | 0.937 | 0.663 | -0.274 [-0.279, -0.269] | -0.513 [-0.536, -0.486] | - |
| 300 | 16384 | 0.7 | sig | 0.820 | 0.634 | -0.186 [-0.195, -0.177] | -0.220 [-0.240, -0.197] | - |
| 300 | 65536 | 0.95 | sig | 1.000 | 0.716 | -0.284 [-0.288, -0.280] | -0.930 [-0.940, -0.921] | - |
| 300 | 65536 | 0.9 | sig | 1.000 | 0.696 | -0.304 [-0.309, -0.298] | -0.939 [-0.948, -0.929] | - |
| 300 | 65536 | 0.8 | sig | 1.000 | 0.664 | -0.336 [-0.341, -0.332] | -0.953 [-0.958, -0.947] | - |
| 300 | 65536 | 0.7 | sig | 0.999 | 0.635 | -0.364 [-0.369, -0.359] | -0.952 [-0.960, -0.943] | - |
| 1000 | 16384 | 0.95 | sig | 0.739 | 0.622 | -0.116 [-0.125, -0.109] | -0.330 [-0.357, -0.301] | - |
| 1000 | 16384 | 0.9 | sig | 0.703 | 0.606 | -0.097 [-0.107, -0.086] | -0.188 [-0.216, -0.160] | - |
| 1000 | 16384 | 0.8 | sig | 0.631 | 0.588 | -0.043 [-0.056, -0.031] | -0.053 [-0.072, -0.034] | - |
| 1000 | 16384 | 0.7 | sig | 0.599 | 0.572 | -0.027 [-0.039, -0.015] | -0.075 [-0.082, -0.068] | - |
| 1000 | 65536 | 0.95 | sig | 1.000 | 0.624 | -0.376 [-0.382, -0.369] | -0.962 [-0.969, -0.956] | - |
| 1000 | 65536 | 0.9 | sig | 0.996 | 0.610 | -0.387 [-0.394, -0.380] | -0.907 [-0.919, -0.895] | - |
| 1000 | 65536 | 0.8 | sig | 0.939 | 0.590 | -0.349 [-0.356, -0.343] | -0.551 [-0.579, -0.525] | - |
| 1000 | 65536 | 0.7 | sig | 0.809 | 0.574 | -0.234 [-0.242, -0.226] | -0.224 [-0.246, -0.202] | - |
| 3000 | 16384 | 0.95 | sig | 0.596 | 0.575 | -0.021 [-0.045, +0.005] | -0.046 [-0.068, -0.024] | - |
| 3000 | 16384 | 0.9 | sig | 0.562 | 0.567 | +0.006 [-0.013, +0.026] | -0.017 [-0.034, +0.001] | - |
| 3000 | 16384 | 0.8 | sig | 0.545 | 0.549 | +0.004 [-0.013, +0.027] | -0.028 [-0.038, -0.019] | - |
| 3000 | 16384 | 0.7 | sig | 0.521 | 0.551 | +0.030 [+0.014, +0.053] | -0.003 [-0.011, +0.003] | WIN |
| 3000 | 65536 | 0.95 | sig | 0.764 | 0.577 | -0.188 [-0.204, -0.167] | -0.084 [-0.095, -0.073] | - |
| 3000 | 65536 | 0.9 | sig | 0.716 | 0.567 | -0.149 [-0.167, -0.129] | -0.259 [-0.272, -0.244] | - |
| 3000 | 65536 | 0.8 | sig | 0.632 | 0.548 | -0.084 [-0.107, -0.063] | -0.078 [-0.090, -0.066] | - |
| 3000 | 65536 | 0.7 | sig | 0.592 | 0.551 | -0.042 [-0.055, -0.026] | -0.098 [-0.106, -0.089] | - |
| 3000 | 262144 | 0.95 | sig | 1.000 | 0.577 | -0.423 [-0.435, -0.409] | -0.974 [-0.979, -0.969] | - |
| 3000 | 262144 | 0.9 | sig | 0.999 | 0.567 | -0.432 [-0.446, -0.417] | -0.963 [-0.971, -0.953] | - |
| 3000 | 262144 | 0.8 | sig | 0.965 | 0.548 | -0.417 [-0.434, -0.400] | -0.726 [-0.748, -0.701] | - |
| 3000 | 262144 | 0.7 | sig | 0.844 | 0.551 | -0.292 [-0.309, -0.278] | -0.333 [-0.356, -0.307] | - |
| 10000 | 65536 | 0.95 | sig | 0.607 | 0.537 | -0.069 [-0.101, -0.045] | -0.052 [-0.072, -0.032] | - |
| 10000 | 65536 | 0.9 | sig | 0.560 | 0.531 | -0.029 [-0.054, -0.003] | -0.077 [-0.108, -0.053] | - |
| 10000 | 65536 | 0.8 | sig | 0.547 | 0.535 | -0.012 [-0.030, +0.007] | +0.007 [-0.007, +0.017] | - |
| 10000 | 65536 | 0.7 | sig | 0.536 | 0.528 | -0.007 [-0.031, +0.017] | -0.009 [-0.032, +0.013] | - |
| 10000 | 262144 | 0.95 | sig | 0.803 | 0.537 | -0.266 [-0.295, -0.237] | -0.184 [-0.248, -0.097] | - |
| 10000 | 262144 | 0.9 | sig | 0.726 | 0.534 | -0.192 [-0.202, -0.178] | -0.313 [-0.345, -0.282] | - |
| 10000 | 262144 | 0.8 | sig | 0.643 | 0.535 | -0.108 [-0.117, -0.099] | -0.114 [-0.134, -0.095] | - |
| 10000 | 262144 | 0.7 | sig | 0.608 | 0.528 | -0.080 [-0.096, -0.055] | -0.114 [-0.145, -0.083] | - |

Cells with CI lower bound > +0.01: 1 (cell = (N,B) with any level)

### K1 near-key membership, locomo keys (AUC; VSA = best of w, kappa tuned on tuning seeds)

| N | B | level | strongest baseline | baseline AUC | VSA AUC | VSA - baseline [95% CI] | VSA TPR@1% - baseline [CI] | win cell |
|---|---|---|---|---|---|---|---|---|
| 100 | 16384 | 0.1 | sig | 0.972 | 0.862 | -0.110 [-0.128, -0.094] | -0.642 [-0.704, -0.577] | - |
| 100 | 16384 | 0.2 | sig | 0.744 | 0.738 | -0.006 [-0.056, +0.040] | -0.163 [-0.206, -0.125] | - |
| 100 | 16384 | 0.3 | sig | 0.541 | 0.614 | +0.073 [+0.029, +0.119] | -0.023 [-0.036, -0.010] | WIN |
| 100 | 65536 | 0.1 | sig | 0.997 | 0.868 | -0.129 [-0.144, -0.114] | -0.817 [-0.867, -0.745] | - |
| 100 | 65536 | 0.2 | sig | 0.933 | 0.740 | -0.193 [-0.238, -0.146] | -0.364 [-0.440, -0.273] | - |
| 100 | 65536 | 0.3 | sig | 0.660 | 0.632 | -0.028 [-0.084, +0.030] | -0.041 [-0.057, -0.021] | - |
| 300 | 16384 | 0.1 | sig | 0.733 | 0.740 | +0.007 [-0.031, +0.041] | -0.137 [-0.155, -0.116] | - |
| 300 | 16384 | 0.2 | sig | 0.550 | 0.649 | +0.099 [+0.036, +0.156] | -0.044 [-0.057, -0.029] | WIN |
| 300 | 16384 | 0.3 | dsbfc | 0.505 | 0.582 | +0.077 [+0.008, +0.145] | -0.010 [-0.019, -0.002] | - |
| 300 | 65536 | 0.1 | sig | 0.972 | 0.753 | -0.218 [-0.254, -0.184] | -0.700 [-0.739, -0.656] | - |
| 300 | 65536 | 0.2 | sig | 0.718 | 0.666 | -0.052 [-0.108, +0.002] | -0.118 [-0.154, -0.087] | - |
| 300 | 65536 | 0.3 | dsbfc | 0.498 | 0.601 | +0.103 [+0.021, +0.182] | -0.010 [-0.016, -0.004] | WIN |

Cells with CI lower bound > +0.01: 3 (cell = (N,B) with any level)

### K2 near-key lookup, synth keys (top-1 accuracy)

| N | B | level | strongest baseline | baseline acc | VSA acc | VSA - baseline [95% CI] |
|---|---|---|---|---|---|---|
| N=300 B=16384 level=0.95 | sig | 0.998 | 0.906 | -0.092 [-0.104, -0.081] |
| N=300 B=16384 level=0.9 | sig | 0.980 | 0.803 | -0.177 [-0.194, -0.161] |
| N=300 B=16384 level=0.8 | sig | 0.849 | 0.587 | -0.263 [-0.282, -0.236] |
| N=300 B=16384 level=0.7 | sig | 0.633 | 0.364 | -0.269 [-0.289, -0.249] |
| N=300 B=65536 level=0.95 | sig | 1.000 | 1.000 | +0.000 [+0.000, +0.000] |
| N=300 B=65536 level=0.9 | sig | 1.000 | 1.000 | +0.000 [+0.000, +0.000] |
| N=300 B=65536 level=0.8 | sig | 1.000 | 0.996 | -0.004 [-0.009, -0.001] |
| N=300 B=65536 level=0.7 | sig | 0.997 | 0.958 | -0.039 [-0.048, -0.028] |
| N=1000 B=16384 level=0.95 | sig | 0.438 | 0.257 | -0.182 [-0.213, -0.143] |
| N=1000 B=16384 level=0.9 | sig | 0.343 | 0.188 | -0.156 [-0.180, -0.130] |
| N=1000 B=16384 level=0.8 | sig | 0.246 | 0.076 | -0.170 [-0.207, -0.136] |
| N=1000 B=65536 level=0.95 | sig | 1.000 | 0.924 | -0.076 [-0.087, -0.067] |
| N=1000 B=65536 level=0.9 | sig | 0.989 | 0.822 | -0.167 [-0.193, -0.153] |
| N=1000 B=65536 level=0.8 | sig | 0.824 | 0.638 | -0.187 [-0.193, -0.180] |
| N=1000 B=65536 level=0.7 | sig | 0.607 | 0.393 | -0.213 [-0.247, -0.193] |

### K3 set similarity (RMSE of Jaccard estimate; ratio = VSA / best b-bit MinHash)

| n | B | J | best MinHash | RMSE | best VSA | RMSE | ratio |
|---|---|---|---|---|---|---|---|
| 100 | 256 | 0.1 | mh4 | 0.0479 | vsa_w2_k2.0 | 0.0379 | 0.79 |
| 100 | 256 | 0.3 | mh2 | 0.0531 | vsa_w1_k1.0 | 0.0833 | 1.57 |
| 100 | 256 | 0.5 | mh2 | 0.0455 | vsa_w4_k2.0 | 0.0851 | 1.87 |
| 100 | 256 | 0.7 | mh1 | 0.0425 | vsa_w4_k3.0 | 0.0546 | 1.29 |
| 100 | 256 | 0.9 | mh1 | 0.0269 | vsa_w8_k1.0 | 0.0276 | 1.02 |
| 100 | 1024 | 0.1 | mh1 | 0.0268 | vsa_w2_k1.0 | 0.0320 | 1.20 |
| 100 | 1024 | 0.3 | mh2 | 0.0251 | vsa_w1_k1.0 | 0.0284 | 1.13 |
| 100 | 1024 | 0.5 | mh1 | 0.0262 | vsa_w1_k1.0 | 0.0346 | 1.32 |
| 100 | 1024 | 0.7 | mh2 | 0.0219 | vsa_w1_k1.0 | 0.0303 | 1.38 |
| 100 | 1024 | 0.9 | mh1 | 0.0141 | vsa_w8_k1.0 | 0.0145 | 1.03 |
| 100 | 4096 | 0.1 | mh8 | 0.0112 | vsa_w1_k1.0 | 0.0128 | 1.14 |
| 100 | 4096 | 0.3 | mh4 | 0.0124 | vsa_w4_k3.0 | 0.0211 | 1.70 |
| 100 | 4096 | 0.5 | mh1 | 0.0135 | vsa_w1_k1.0 | 0.0183 | 1.35 |
| 100 | 4096 | 0.7 | mh1 | 0.0123 | vsa_w1_k1.0 | 0.0152 | 1.24 |
| 100 | 4096 | 0.9 | mh2 | 0.0057 | vsa_w8_k1.0 | 0.0071 | 1.25 |
| 1000 | 256 | 0.1 | mh8 | 0.0512 | vsa_w2_k1.0 | 0.0532 | 1.04 |
| 1000 | 256 | 0.3 | mh1 | 0.0589 | vsa_w1_k1.0 | 0.0680 | 1.15 |
| 1000 | 256 | 0.5 | mh2 | 0.0620 | vsa_w1_k1.0 | 0.0638 | 1.03 |
| 1000 | 256 | 0.7 | mh1 | 0.0387 | vsa_w4_k3.0 | 0.0538 | 1.39 |
| 1000 | 256 | 0.9 | mh1 | 0.0340 | vsa_w4_k2.0 | 0.0247 | 0.73 |
| 1000 | 1024 | 0.1 | mh4 | 0.0250 | vsa_w2_k1.0 | 0.0321 | 1.29 |
| 1000 | 1024 | 0.3 | mh4 | 0.0243 | vsa_w4_k2.0 | 0.0418 | 1.72 |
| 1000 | 1024 | 0.5 | mh1 | 0.0253 | vsa_w1_k1.0 | 0.0366 | 1.45 |
| 1000 | 1024 | 0.7 | mh2 | 0.0173 | vsa_w1_k1.0 | 0.0298 | 1.72 |
| 1000 | 1024 | 0.9 | mh1 | 0.0156 | vsa_w1_k1.0 | 0.0164 | 1.05 |
| 1000 | 4096 | 0.1 | mh4 | 0.0130 | vsa_w4_k2.0 | 0.0160 | 1.23 |
| 1000 | 4096 | 0.3 | mh2 | 0.0138 | vsa_w1_k1.0 | 0.0169 | 1.23 |
| 1000 | 4096 | 0.5 | mh1 | 0.0148 | vsa_w1_k1.0 | 0.0157 | 1.06 |
| 1000 | 4096 | 0.7 | mh1 | 0.0114 | vsa_w1_k1.0 | 0.0158 | 1.39 |
| 1000 | 4096 | 0.9 | mh1 | 0.0070 | vsa_w8_k1.0 | 0.0076 | 1.08 |

Cells where VSA RMSE is >10% lower: 2 of 30

### K4b: single missing role, bits per record below and above the information content (R=6, V=256)

| cell | table_subset | bloomtriples | vsa | strongest baseline | VSA - baseline [95% CI] |
|---|---|---|---|---|---|
| N=200 bits_mul=0.0625 | 0.062 | 0.005 | 0.098 | table_subset | +0.035 [+0.015, +0.058] |
| N=200 bits_mul=0.125 | 0.125 | 0.006 | 0.215 | table_subset | +0.090 [+0.072, +0.108] |
| N=200 bits_mul=0.25 | 0.250 | 0.010 | 0.483 | table_subset | +0.232 [+0.205, +0.260] |
| N=200 bits_mul=0.5 | 0.500 | 0.027 | 0.843 | table_subset | +0.343 [+0.332, +0.354] |
| N=200 bits_mul=0.75 | 0.750 | 0.069 | 0.956 | table_subset | +0.206 [+0.192, +0.219] |
| N=200 bits_mul=1.0 | 1.000 | 0.182 | 0.993 | table_subset | -0.007 [-0.012, -0.003] |
| N=200 bits_mul=2.0 | 1.000 | 0.928 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| N=800 bits_mul=0.0625 | 0.062 | 0.005 | 0.100 | table_subset | +0.038 [+0.028, +0.050] |
| N=800 bits_mul=0.125 | 0.125 | 0.006 | 0.210 | table_subset | +0.085 [+0.072, +0.100] |
| N=800 bits_mul=0.25 | 0.250 | 0.010 | 0.497 | table_subset | +0.247 [+0.226, +0.265] |
| N=800 bits_mul=0.5 | 0.500 | 0.027 | 0.853 | table_subset | +0.353 [+0.336, +0.367] |
| N=800 bits_mul=0.75 | 0.750 | 0.069 | 0.962 | table_subset | +0.212 [+0.203, +0.220] |
| N=800 bits_mul=1.0 | 1.000 | 0.182 | 0.993 | table_subset | -0.007 [-0.010, -0.003] |
| N=800 bits_mul=2.0 | 1.000 | 0.925 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| N=1600 bits_mul=0.0625 | 0.062 | 0.005 | 0.082 | table_subset | +0.020 [+0.013, +0.027] |
| N=1600 bits_mul=0.125 | 0.125 | 0.006 | 0.214 | table_subset | +0.089 [+0.074, +0.103] |
| N=1600 bits_mul=0.25 | 0.250 | 0.010 | 0.488 | table_subset | +0.238 [+0.215, +0.259] |
| N=1600 bits_mul=0.5 | 0.500 | 0.027 | 0.837 | table_subset | +0.337 [+0.332, +0.342] |
| N=1600 bits_mul=0.75 | 0.750 | 0.069 | 0.963 | table_subset | +0.212 [+0.202, +0.223] |
| N=1600 bits_mul=1.0 | 1.000 | 0.183 | 0.994 | table_subset | -0.007 [-0.012, -0.001] |
| N=1600 bits_mul=2.0 | 1.000 | 0.932 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |

### K4c: u unknown roles, strongest conventional = parity-address tables (pat) or exact table

| cell | table_subset | pat | vsa | strongest baseline | VSA - baseline [95% CI] |
|---|---|---|---|---|---|
| R=6 u=1 N=200 bits_per_rec=6 | 0.125 | 0.410 | 0.196 | pat | -0.215 [-0.258, -0.171] |
| R=6 u=1 N=200 bits_per_rec=12 | 0.250 | 0.765 | 0.471 | pat | -0.294 [-0.329, -0.258] |
| R=6 u=1 N=200 bits_per_rec=24 | 0.500 | 0.998 | 0.821 | pat | -0.177 [-0.223, -0.135] |
| R=6 u=1 N=200 bits_per_rec=36 | 0.750 | 0.998 | 0.965 | pat | -0.033 [-0.056, -0.012] |
| R=6 u=1 N=200 bits_per_rec=48 | 1.000 | 1.000 | 0.990 | table_subset | -0.010 [-0.019, -0.002] |
| R=6 u=1 N=200 bits_per_rec=60 | 1.000 | 1.000 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| R=6 u=1 N=200 bits_per_rec=96 | 1.000 | 0.998 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| R=6 u=1 N=800 bits_per_rec=6 | 0.125 | 0.383 | 0.192 | pat | -0.192 [-0.231, -0.154] |
| R=6 u=1 N=800 bits_per_rec=12 | 0.250 | 0.765 | 0.506 | pat | -0.258 [-0.294, -0.225] |
| R=6 u=1 N=800 bits_per_rec=24 | 0.500 | 0.992 | 0.840 | pat | -0.152 [-0.173, -0.131] |
| R=6 u=1 N=800 bits_per_rec=36 | 0.750 | 0.996 | 0.977 | pat | -0.019 [-0.033, -0.006] |
| R=6 u=1 N=800 bits_per_rec=48 | 1.000 | 0.992 | 0.996 | table_subset | -0.004 [-0.010, +0.000] |
| R=6 u=1 N=800 bits_per_rec=60 | 1.000 | 0.998 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| R=6 u=1 N=800 bits_per_rec=96 | 1.000 | 0.996 | 1.000 | table_subset | +0.000 [+0.000, +0.000] |
| R=6 u=2 N=200 bits_per_rec=6 | 0.125 | 0.098 | 0.019 | table_subset | -0.106 [-0.115, -0.098] |
| R=6 u=2 N=200 bits_per_rec=12 | 0.250 | 0.256 | 0.079 | pat | -0.177 [-0.254, -0.115] |
| R=6 u=2 N=200 bits_per_rec=24 | 0.500 | 0.429 | 0.348 | table_subset | -0.152 [-0.190, -0.112] |
| R=6 u=2 N=200 bits_per_rec=36 | 0.750 | 0.667 | 0.677 | table_subset | -0.073 [-0.110, -0.035] |
| R=6 u=2 N=200 bits_per_rec=48 | 1.000 | 0.850 | 0.896 | table_subset | -0.104 [-0.129, -0.079] |
| R=6 u=2 N=200 bits_per_rec=60 | 1.000 | 1.000 | 0.956 | table_subset | -0.044 [-0.065, -0.023] |
| R=12 u=2 N=200 bits_per_rec=24 | 0.250 | 0.729 | 0.367 | pat | -0.363 [-0.421, -0.308] |
| R=12 u=2 N=200 bits_per_rec=36 | 0.375 | 0.894 | 0.663 | pat | -0.231 [-0.256, -0.202] |
| R=12 u=2 N=200 bits_per_rec=48 | 0.500 | 0.975 | 0.875 | pat | -0.100 [-0.133, -0.067] |
| R=12 u=2 N=200 bits_per_rec=60 | 0.625 | 1.000 | 0.969 | pat | -0.031 [-0.050, -0.017] |
| R=12 u=1 N=200 bits_per_rec=12 | 0.125 | 0.852 | 0.492 | pat | -0.360 [-0.419, -0.306] |
| R=12 u=1 N=200 bits_per_rec=24 | 0.250 | 1.000 | 0.858 | pat | -0.142 [-0.177, -0.110] |
| R=12 u=1 N=200 bits_per_rec=48 | 0.500 | 0.996 | 0.988 | pat | -0.008 [-0.019, +0.004] |


# Round 2: many unknown roles and real-text membership

Pre-registered in `benchmarks/results/factstore_kill_20261114/PREREG.md` (commit bf620e4, amended before any report seed in 7dae104). Results, tables and harness: `benchmarks/results/factstore_kill_20261114/round2/`, `scripts/factstore_kill/round2/`. Hardware: GCP n2-standard-8, single-threaded timing where stated. Deviations from the registration are listed at the end; they limit the tested range and are not hidden.

## Lead A: R >= 12 roles, u >= 3 unknown roles, arbitrary query subsets

Task: N records, V=256 fillers per role, random u-subset unknown per query, return all u fillers exactly. Arms at equal total bits: resonator-decoded superposed record store (VSA-res), exact column store (COL), per-role inverted index with measured Elias-Fano cost (INV), per-record token Bloom filter (ROWBF), static binary-fuse fingerprint filter over full records decoded by enumerating V^u tuples (FPF; model validated against the real `xorf` crate), parity-address tables (PAT, analytic).

Measurement, N=256, R=12, 20 report seeds, 100 queries per seed. Bits per record needed to reach accuracy 0.99 (grid 12..192; "n/r" = not reached):

| (R,u) | VSA-res | COL | INV | ROWBF | FPF (model) | PAT (analytic) |
|---|---|---|---|---|---|---|
| (12,3) | n/r (accuracy 0.00 at 192) | 96 | 128 | n/r | 40 | 128 |
| (12,4) | n/r | 96 | 128 | n/r | 48 | n/r |
| (12,6) | n/r | 96 | 128 | n/r | 64 | n/r |
| (16,3) | n/r | 128 | 160 | n/r | 40 | 128 |

At 0.90 the thresholds are the same or 8 bits lower for FPF. VSA-res scored below 0.02 on the tuning seeds at 192 bits per record (D=49,152 at w=1) in every run cell, so by the registered monotone rule every lower grid point is dead. Criterion A: no bits win in any cell. Not run: (16,4), (16,8), N=1024, N=4096, the scaling probes and the timing runs (call budget and wall clock). The resonator pilot on tuning seeds (`dbg_a.log`, R=12, u=3) gave accuracy 0.44 at N=1, D=16,384; 0.19 at N=4, D=65,536; 0.00 at N=16, D=131,072, and the true tuple scored higher than the returned one in every failure, so the search, not the store, is what fails there. The registered strong-search rerun (16 restarts, 100 iterations) was not run because tuned accuracy was below its 0.05 trigger; the exhaustive-decode oracle below separates the store from the search instead.

Oracle (all 256^3 tuples scored against the same bundle, N=64, R=12, u=3, 2 seeds, 8 queries; measurement): accuracy 0.31 at 32 bits per record, 0.94 at 64, 1.00 at 128 (w=1 counters; w=2 is worse: 0.06, 0.25, 1.00), at 1.3 to 4.3 s per query. So even with a perfect search the superposed store needs about 64 to 128 bits per record at N=64, against 96 for the exact column store and about 36 to 40 for the fingerprint filter, and the cost of an exhaustive decode is V^u dot products of length D.

Real filter check (`fpf.jsonl`, xorf 0.11, MIT): BinaryFuse8/16 at n=20,000 keys use 9.83/19.66 bits per key (overhead 1.23, the model assumes 1.13); measured false-positive rates 3.90e-3 and 1.55e-5 match 2^-8 and 2^-16; enumerating the 16.7M tuples of a u=3 query took 136 to 154 ms under a loaded machine. u=4 would be extrapolated (about 256 times) and u>=5 is infeasible by enumeration. These timings were taken while other jobs ran and are upper bounds.

Verdict A (inference from the measurements above): clean kill for the registered range. At R>=12, u>=3 a plain column store is exact at 8R bits and fast, the static fingerprint filter supports arbitrary subsets at about 1.13(u log2 V + 7) bits per record, and the resonator-decoded superposition did not reach 0.02 accuracy at 192 bits per record. A win for VSA on bits is not supported at any measured point, so the query-time axis never has to be traded against it. Where the filter's V^u enumeration becomes infeasible (u>=5), the column store still answers exactly at 96 to 128 bits per record in microseconds to milliseconds.

## Lead B: real-text near-key membership

Cells that round 1 reported: C1 = LoCoMo, N=100, B=2^14, 30% character substitution; C2 = N=300, B=2^14, 20%; C3 = N=300, B=2^16, 30%. All three used centred hashed char-3-gram TF-IDF keys (d=2048), noise was uniform character substitution, load was 164 to 655 bits per fact, and the strongest round-1 baselines were at AUC about 0.5. The noisy members were scored against clean non-members.

Round-2 measurement (LoCoMo 6 report seeds, memory-lme-dev user-turn sentences 5 report seeds; 1000 positives and 4000 negatives per seed; N in {100,300,1000,3000}; B in {2^14,2^16}; character substitution 0.1/0.2/0.3 and word deletion 0.2/0.4/0.6; 96 cells; arms: VSA, SimHash signatures, distance-sensitive Bloom, global shingle Bloom, per-fact shingle Bloom, count-min over shingles, b-bit MinHash; encoders c3, c5, w12). All cells are in `b_tables_locomo.md` and `b_tables_lme.md`.

- Protocol M (non-members corrupted the same way, the registered primary): VSA AUC win (CI lower bound above +0.01) with TPR@1% difference lower bound above 0 in 6 of 48 LoCoMo cells and 3 of 48 lme cells, all at substitution 0.1 to 0.3 with 16,384 to 65,536 bits and N=300 to 3000, and every one at near-chance operating points: VSA AUC 0.56 to 0.65, TPR@1% FPR 0.017 to 0.039 (LoCoMo) and 0.019 to 0.026 (lme), TPR@0.1% FPR at most 0.007. The gains over the best baseline are +0.04 to +0.07 AUC and +0.004 to +0.013 TPR@1%. The registered usefulness clause (VSA TPR@1% at least 0.10 in the winning cells) fails in every winning cell.
- C1 to C3 under protocol M: C1 AUC +0.018 but TPR@1% -0.107; C2 AUC +0.020 (CI lower bound +0.008, below the margin) and TPR@1% -0.074; C3 AUC -0.023 and TPR@1% -0.129. Not wins. The round-1 edge came from comparing against baselines that collapse when clean non-members outscore corrupted members: under protocol C the best baselines reach AUC near 0.01 to 0.2 in these cells (corrupted members score below clean strangers), and the global shingle Bloom, count-min and MinHash baselines added in round 2 reach AUC 0.60 to 0.73 in the same cells.
- Control K (corrupted vs clean non-members): VSA 0.50 to 0.52, i.e. the bundle does not detect corruption; several baselines are well below 0.5 on K, which is why protocol C is not a fair test for them.
- Where VSA loses: 39 of 48 LoCoMo cells and 45 of 48 lme cells have no VSA win; at low noise and N<=300 b-bit MinHash and SimHash signatures are at AUC 0.99 to 1.00 with TPR@1% above 0.9 where VSA is at 0.83 to 0.99 and 0.2 to 0.8.

Verdict B: kill. The AUC edge in the near-chance cells is real in the sense that it survives noise-matched non-members, more seeds and the shingle-Bloom, count-min and MinHash baselines, but it sits at TPR@1% FPR of 2 to 4%, which no user can operate at; it fails the registered usefulness clause and the round-1 cells themselves are not wins. Cells with a 99.9% interval view give the same count (6 and 3).

## Deviations and limits (all reduce the tested range)

- Lead B used 6 LoCoMo and 5 lme report seeds, not 20 (the jobs ran about 17 minutes per seed and were stopped at the call budget); B=2^18 was not run; streaming counters were not measured. With 5 to 6 seeds the intervals are wider than registered and 0.1% FPR figures rest on 20 to 24 pooled negative events.
- Lead A: N=1024, N=4096, (16,4), (16,8), the scaling probes, the strong-search rerun, the oracle at N=256 and the timing runs were not completed. FPF accuracy is a model validated at 8 and 16 bit fingerprints; the 32-bit filter's false-positive rate is not measurable at the probe count used.
- Query-time comparisons for the VSA arm are therefore not reported; the oracle's seconds per query and the filter's measured 0.14 s are the only timings.

## Where the fact store is described as a space, speed or capacity advantage

Searched README.md, docs/ (including docs/overrides website copy) for fact-store, vector-symbolic, bundle and holographic wording near faster, smaller, capacity, advantage, compress and graceful. Most pages already state the measured limits (docs/concepts/capacity.md lines 70-96 say the bundle ties a Bloom filter and has no accuracy advantage on key-value retrieval). Lines that can be read as an advantage:

- docs/concepts/holographic.md:2 and :63 (frontmatter and the section "Graceful degradation, measured"): graceful degradation is a property of damaged vectors, not an advantage over a conventional structure at equal bits.
- docs/SUPERPOSITION-FLOOR.md:9-13 and :214-217, :222-229: "coupled-encoder advantage" and "capacity beyond the floor" are synthetic encoder results, stated there as not a database compression claim; the round-2 oracle shows the store at about 64 to 128 bits per record for exact three-role recovery.
- docs/PREREGISTRATION-binding-readout.md:317 and :1026: "verifies role-filler bindings at higher d' than dense HRR/MAP at matched D" and "holographic properties" are comparisons between VSA variants, not against conventional structures.
- docs/COMPARISON.md:12 and docs/concepts/index.md:8 list the store as the tool for "structured facts" and "relational and multi-hop queries" without a space or speed claim.

No page in README or the website copy states that the fact store is faster or smaller than a conventional structure. Proposed wording, for the owner to decide: "The vector-symbolic layer is an algebraic interface for composing and querying role-filler records (bind, bundle, unbind, merge by elementwise sum). At equal bits it ties a Bloom filter or count-sketch on membership and loses to stored signatures, b-bit MinHash, column stores and static fingerprint filters on retrieval; it offers no space, speed or capacity advantage over them."
