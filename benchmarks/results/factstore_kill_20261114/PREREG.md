# Fact-store win search: pre-registration (written 2026-11-14 before any measurement)

Repo: writerslogic/warm main 14ca7ff3. Branch feat/fact-store-win. Harness: kill/kill_tests.py (numpy). Seeds: tuning seeds 0-1, report seeds 100-119 (20, paired: same keys, queries, projection for every arm). Real-data cells use 2 tuning + 10 report seeds. Cells run: B=2^14 with N<=3000, B=2^16 with all N, B=2^18 with N>=3000 (other cells are all-ties or below 1 bit/fact).

## Equivalence kills (no compute; recorded as outcome "tie by construction")
- bundle_bloom (OR of sparse indices) is a Bloom filter; counting bundle is a counting Bloom filter; dense bipolar bundle is a count-sketch/AMS projection; BlockCodeBundle is a partitioned counting Bloom filter.
- (i) merge by addition: Bloom OR, CMS add, IBLT add, HLL max already merge without raw facts. Equivalent, so tie.
- (iv) O(1) online update/delete: counting Bloom, cuckoo and quotient filters already do it, at 12-20 bits/item for 1% FPR; existing bundle_k_from_n_20261114.json shows the counting arm needs 32 bits/counter to match a plain Bloom. Dominated, no compute.
- (v) privacy: noised counting bundle is a Gaussian-noised count-min sketch (standard DP sketch). Equivalent, no compute.
- (vi-b) set similarity under noisy tokens by bundle-of-signatures cosine equals SimHash of the summed embedding (random projection is linear). Equivalent for the linear encoder; measured only where the per-item sign nonlinearity differs (K3).

## Measured candidates
All arms use the SAME sign-random-projection encoder of the key (dim d, Gaussian projection). The projection matrix is not counted for any arm. Bits count only the stored structure.

K1 near-key membership ("have I stored a fact like this?"). Members: stored key x_i queried with a noisy copy q (cosine rho to x_i). Non-members: fresh unrelated keys. Metric: AUC and TPR at 1% FPR. Keys: (a) synthetic Gaussian d=128; (b) real: LoCoMo locomo10.json "observation" sentences, hashed char-3gram TF-IDF (d=2048), queries = same sentence with 10/20/30% character substitutions; non-members = other sentences.
 VSA arm: counters w in {1,2,3,4,8} bits, D=B/w counters, bundle of sign codes, score = <S, sign code of q>. Static-quantised (best case) and streaming-saturating (online insert) both measured; the kill rule uses the weaker.
 Baselines at equal total bits B:
  B1 stored sign signatures, m=floor(B/N) bits each, score = max agreement over items (this is also "ANN over fact embeddings at equal bits" in its best 1-bit form; full-precision ANN holds only floor(B/(32d)) items, reported analytically as an upper bound on recall).
  B2 distance-sensitive Bloom filter: L bands of r projection bits hashed as tokens into a Bloom of B bits with k probes, score = fraction of the L tokens present; (r,L,k) grid-tuned per cell on tuning seeds (generous).
  B3 counting variant of B2 with 4-bit counters (B/4 cells) for the delete-capable comparison.
 Hyperparameters of every arm (w for VSA; m is forced for B1; r,L,k for B2) are tuned per cell on the tuning seeds and frozen for the report seeds.
 B1 also tries storing only a fraction f in {1,.5,.25,.125} of the items with longer signatures (graceful degradation). VSA static build quantises the sum to w bits with clip at kappa*sqrt(N), kappa in {1,2,3}. Sweep: B in {2^14, 2^16, 2^18} bits; N in {100,300,1000,3000,10000,30000}; rho in {0.95,0.9,0.8,0.7}. Bits/fact ranges 0.5 to 2600, so the cliff of each arm is located.
K2 near-key lookup (return a 10-bit value id for a noisy key). VSA: bundle of bind(sign code, value atom), cleanup over V=1024 atoms; baselines: stored signature (m bits) + 10-bit id with max-agreement scan; T-table LSH functions (idealised Bloomier at 1.23 x 10 bits per item per table, majority vote, best T tuned). Metric: top-1 value accuracy; same sweep reduced (B in {2^16,2^18}).
K3 set similarity of sessions: (a) exact-token sets, estimate Jaccard, metric RMSE, VSA count-sketch bundle (w bits, cosine to Jaccard) vs b-bit MinHash (b in 1,2,4,8; k=B/b) at B in {256,1024,4096}, J in {0.1,0.3,0.5,0.7,0.9}, n in {100,1000}; (b) noisy copies (facts perturbed rho=0.9): VSA bundle-of-signatures cosine vs MinHash over LSH-band tokens vs SimHash of mean embedding, metric: rank correlation with true set overlap.
K4 partial-cue role-filler retrieval: N records, R=6 roles, V=256 fillers/role; cue = random subset of 2..4 roles (and variant with 1 corrupted role); retrieve the missing filler. VSA: bundle of bound role-filler records + cleanup (existing semantics) vs exact table + (role,filler) inverted index (N*R*8 bits plus postings 2*log2(N) bits per entry). Equal bits and equal query time reported. Information bound: table cannot be beaten on exact retrieval at equal bits; measured to record the cliff.

## Kill rule (applies per candidate)
A candidate survives only if, over the 20 report seeds, the paired difference (VSA minus best baseline, each baseline at its tuned setting) in the task metric has a 95% bootstrap CI whose lower bound exceeds +0.01 AUC (K1), +0.02 accuracy (K2), a >10% lower RMSE (K3), or +0.02 accuracy (K4), in at least 3 cells (distinct (B,N) pairs) at 2 or more noise levels, on BOTH the synthetic and the real keys where both exist. A tie (CI contains 0) is "tie", not a win. The survivor must still win with the streaming-saturating counters if online insertion is claimed.
A hybrid counts only if the ablation without the superposed part loses.
