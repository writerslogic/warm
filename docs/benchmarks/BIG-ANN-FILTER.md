# big-ann-benchmarks filtered track (YFCC-10M): development results

Status: dated development and pinned-commit harness runs are recorded below. The track
maintainers evaluate the leaderboard on Azure D8lds_v5 (8 vCPU Xeon Platinum 8370C);
none of these different-host measurements establishes an official standing.

## Task

yfcc-10M: 10M CLIP vectors, 192-d uint8, L2; every item has a bag of tags (200,386-tag
vocabulary, 10.8 tags per item on average). 100k queries, each with 1 or 2 tags (61.6% / 38.4%)
that must all be in the item's bag. Ranked by QPS at recall@10 >= 0.90 on the public queries.
Live board (upstream 89a3aba, fetched 2026-10-23): pinecone 85,491 (closed), zilliz 84,596
(closed), parlayivf 37,902, puck 19,193, hwtl_sdu_anns_filter 15,059 (closed), wm_filter 14,468,
dhq 13,671, fdufilterdiskann 5,680, pyanns 5,185, faissplus 3,777, faiss 3,033, cufe 2,917.

Exact match counts of the public queries (from the base metadata):

| matches | queries | 1-tag | 2-tag |
|---|---|---|---|
| < 1k | 24,055 | 8,051 | 16,004 |
| 1k - 10k | 22,800 | 4,058 | 18,742 |
| 10k - 100k | 23,443 | 19,815 | 3,628 |
| 100k - 1M | 24,044 | 24,044 | 0 |
| >= 1M | 5,658 | 5,658 | 0 |

## Index (`core::qgraph::filter::FilteredIndex`)

Per-tag sorted posting lists, membership bitsets for tags with at least `bitset_min` members, and
for every tag with more than `ivf_min` members a k-means inverted file over that tag's members
alone (about `cluster_size` per list, at most `max_lists`); tags with at least `copy_min` members
also keep their rows copied in list order. A query with at most `brute_max` matches (one tag: its
posting; two tags: the intersection, computed when the rarer tag has at most `intersect_max`
members) is scored exactly over every match; otherwise the rarer tag's lists are probed
nearest-centroid first, skipping members without the other tag, until `target` matches are
scored. Distances are exact integer L2 from the i8 dot kernels (AVX-VNNI / AVX512-VNNI / AVX2 /
NEON). With `brute_max = target = usize::MAX` results equal exact filtered brute force (tests,
checked on the scalar, AVX2 and AVX-VNNI tiers; the AVX512-VNNI tier is not verified on hardware).

## Harness runs (public queries)

Intel Core Ultra 9 185H, `taskset -c 0-7` (4 P-cores x 2 hyperthreads), 8 search threads,
`run.py --nodocker --neurips23track filter --dataset yfcc-10M --runs 3`, under the timing lock but
on a host shared with other jobs (load average 18-23 on 22 threads). Build ivf_min 5000,
cluster_size 1000, max_lists 2048, copy_min 50000. Results:
`benchmarks/results/bigann_filter_yfcc10m_dev_2026-10-23.json`.

| run | WARM commit | bitset_min | brute_max | target | recall@10 | QPS |
|---|---|---|---|---|---|---|
| 1 | 372cddb | 5000 | 5000 | 2000 | 0.8857 | 17,530 |
| 1 | 372cddb | 5000 | 5000 | 2500 | 0.9045 | 14,130 |
| 1 | 372cddb | 5000 | 10000 | 2500 | 0.9067 | 16,260 |
| 1 | 372cddb | 5000 | 5000 | 3000 | 0.9194 | 14,232 |
| 1 | 372cddb | 5000 | 5000 | 4000 | 0.9397 | 12,416 |
| 2 | fb28403 | 20000 | 10000 | 2000 | 0.8892 | 17,413 |
| 2 | fb28403 | 20000 | 5000 | 2500 | 0.9045 | 16,205 |
| 2 | fb28403 | 20000 | 10000 | 2500 | 0.9067 | 15,505 |
| 2 | fb28403 | 20000 | 20000 | 2500 | 0.9106 | 13,801 |
| 2 | fb28403 | 20000 | 10000 | 3000 | 0.9208 | 15,940 |

Index size reported by the harness: run 1 10.6 GB, run 2 9.8 GB (bitsets 3.4 GB -> 0.7 GB).
Build 167-179 s. Run-to-run noise on this shared host is large (e.g. target 3000 measured faster
than target 2500 in run 2); treat differences under ~15% as noise.

Development QPS at recall@10 >= 0.90 was about 16k on 4 hyperthreaded P-cores.
No other entry was run on this host, so these measurements do not establish a matched comparison.

## Parameter sweeps (private queries, tuning)

`query.private.2727415019` with its ground truth, the same 8 CPUs, without the lock, load
average in brackets. QPS per band times that band's queries as one batch.

| sweep | build | brute_max:target | recall | QPS | recall by band (<1k, 1k-10k, 10k-100k, 100k-1M, >=1M) |
|---|---|---|---|---|---|
| r1 [~15-22] | ivf_min 20000, bitset_min 20000, max_lists 1024, no copies, routing by rarer posting | 20000:2000 | 0.8903 | 12,279 | 1.0, 0.998, 0.879, 0.751, 0.642 |
| r1 | | 20000:5000 | 0.9521 | 8,221 | 1.0, 1.0, 0.964, 0.885, 0.798 |
| r1 | | 20000:10000 | 0.9799 | 5,762 | 1.0, 1.0, 0.991, 0.950, 0.895 |
| r1 | | 50000:5000 | 0.9557 | 7,151 | 1.0, 1.0, 0.980, 0.885, 0.798 |
| r1 | | 50000:10000 | 0.9805 | 4,978 | 1.0, 1.0, 0.994, 0.950, 0.895 |
| r1 | | 10000:5000 | 0.9521 | 7,283 | as 20000:5000 |
| r2 [10-12] | ivf_min 5000, bitset_min 5000, max_lists 2048, copy_min 50000, routing by match count | 5000:1500 | 0.8587 | 23,788 | 1.0, 0.976, 0.819, 0.702, 0.627 |
| r2 | | 5000:2500 | 0.9043 | 18,336 | 1.0, 0.990, 0.892, 0.785, 0.715 |
| r2 | | 5000:4000 | 0.9396 | 14,179 | 1.0, 0.998, 0.944, 0.856, 0.795 |
| r2 | | 10000:2500 | 0.9065 | 18,768 | 1.0, 1.0, 0.892, 0.785, 0.715 |
| r3 [14-29] | as r2 with bitset_min 50000, copy_min 20000 | 1000:2500 | 0.9042 | 5,686 | 1.0, 0.990, 0.892, 0.785, 0.715 |
| r3 | | 2500:2500 | 0.9042 | 6,459 | |
| r3 | | 5000:2500 | 0.9043 | 13,659 | |
| r3 | | 2500:3500 | 0.9308 | 9,289 | 1.0, 0.996, 0.932, 0.838, 0.772 |

r3 ran at load average up to 29 and is not comparable for QPS; its recall column stands.

## Pinned run (af8d4017, Modal 8 vCPU, 16 GiB, 2026-11-06)

The submission image (`neurips23/filter/warm`, PR harsha-simhadri/big-ann-benchmarks#356, harness
68b8ba5, `ARG WARM_COMMIT=af8d4017`) was run with `run.py --nodocker
--neurips23track filter --algorithm warm --dataset yfcc-10M`, on a Modal function with
`cpu=(8,8) memory=(16384,16384)`: AMD EPYC Zen 3, 8 logical CPUs, no AVX-512, gVisor. Build
136.0 s, index 9.81 GB as reported by the harness, `run.py` 257 s, public queries.
Results: `benchmarks/results/bigann_filter_yfcc10m_pinned_v1.json`.

| brute_max:target | recall@10 | QPS |
|---|---|---|
| **5000:2500** | **0.9044** | **33,623** |
| 10000:3000 | 0.9208 | 32,724 |
| 5000:4000 | 0.9397 | 31,276 |
| 10000:2500 | 0.9067 | 30,875 |
| 5000:3000 | 0.9194 | 23,410 |
| 20000:2500 | 0.9106 | 19,639 |

The CPU model, AVX-512 availability, runtime and core/thread mapping differ from the
evaluation VM. The raw summary retains historical placement projections; those projections
are unsupported as rankings under matched conditions and are not current standings.

## Ice Lake tuning (2026-11-09): held-out selection, ablation, official protocol

Hardware: GCP n2-custom-8-16384 (Intel Xeon Ice Lake-SP, AVX-512 VNNI, 8 vCPU, 16 GiB), a pd-standard
clone of the reference VM. This is the evaluation CPU class, not the Azure D8lds_v5 evaluation VM
itself. Raw data: `benchmarks/results/bigann_filter_yfcc10m_icelake_2026-11-09.json`.

### Profile of the pinned entry (control)

Control = WARM `af8d4017`, build `ivf_min 5000, cluster_size 1000, max_lists 2048, bitset_min 20000,
copy_min 50000`, search `brute_max 10000, target 2500`. On the selection split it scores 24,187
QPS at recall 0.9067 (three further samples 23.8-24.8k).

- 61.6% of the public queries carry one tag, 38.4% two. The median rarest tag has 28,467 members;
  30% of queries have a rarest tag of at most 10,000 members (exact scan), 70% go to the per-tag
  inverted file or to the intersection route.
- Concentration: 6,537 distinct rarest tags over 50,000 queries; the top 10 tags cover 17% of the
  queries and the top 100 cover 41%. Across all 100,000 queries only 19,078 of the 38,374 two-tag
  queries have a distinct label set (half repeat), and the 61,626 one-tag queries use 4,852 tags.
- Time: two-tag queries are 63% of the time. The rarest-tag band of 20,000 to 50,000 members alone
  is 30.5% of the time and runs at 9,245 QPS, scoring only 3,275 rows per query. The cost there is
  the walk over a 20k-200k-id posting list with a random bitset probe per id, not the distance
  kernel. One-tag inverted-file queries run at 23k-60k QPS.
- The scan is memory-bound: about 90-120 ns per scored row per thread (roughly 17 GB/s of
  random 192-byte rows over 8 threads). The VNNI kernel was already in the pin (`tier::VNNI` on
  Ice Lake). A 512-bit `vpdpbusd` variant (`wide`) changes QPS by +3% alone and 0 to -3% inside
  the elite, i.e. within noise (3-run spread about 2-3%), and is off by default.

### Candidates and held-out protocol

Selection used only the "sel" split: a seeded permutation (`numpy.random.default_rng(20261108)`) of
the 100,000 public queries, first 50,000 sorted. The other 50,000 ("conf") and the full set were not
used for any choice. Recall is `get_recall_values` with distance ties on the matching GT rows.
QPS is the best of 3 (search) or 5 (ablation) runs of one `index.search` call with 8 threads.
Fitness for the evolutionary search was QPS if recall >= 0.9015 on sel (0.90 plus about one sigma of
a 50,000-query mean), else QPS scaled by (recall/0.9015)^50. The final configuration is confirmed
last, on all public queries, by the unchanged `run.py`.

Result-preserving execution choices (identical ids and distances; tested against all 32
combinations with and without deletes and bitsets): `sort_queries` (queries of a batch grouped by
rarest, then second label), `block_filter` (the other labels tested 64 ids at a time without
branching with the next block's bitset words prefetched), `skip_tombstones`, `cache_intersections`
(a repeated label set reuses the last intersection), `prefetch` distance, `wide`.

Single-component test (all at brute_max 10000, target 2500, recall 0.9067, sel, 5 runs):
control 24.1k; sort 27.4k; block 29.7k; tombstone skip 25.2k; wide 24.7k; sort+block 33.8k;
sort+block+tombstone 34.1k; sort+block+wide 32.9k; all four 33.9k. Prefetch distance 0 / 8 / 24 /
48 / 96: 21.0k / 22.8k / 23.9k / 23.9k / 22.7k.

Evolution E1 and E1b (lineage in the JSON): mutate brute_max, target, size-banded targets,
intersect_max, min_lists, cache, wide, prefetch; population 10, top 3 kept, two generations without
a >2% gain stop it. E1 reached 38.8k at recall 0.9045 (brute_max 5000, target 2500, intersect_max
1,000,000, prefetch 32); E1b converged to the same genotype (39.6k) in two generations. The
population collapsed to one genotype, so the lack of further gain says the neighbourhood is flat,
not that nothing better exists. Size-banded targets, min_lists, wide and edge changes never won.

Build variants (sel, elite execution choices): `cluster_size 500, max_lists 4096` raises recall per
scored row (target 2500 gives 0.9336), so the target drops to 1750: 45,282 QPS at 0.9069 (targets
1000/1250/1500/1750/2000: 53.4k@0.856, 50.9k@0.877, 47.9k@0.894, 45.3k@0.907, 43.2k@0.918). Build
417 s against 155 s; index 9.34 GiB both. `bitset_min 5000` and `copy_min 15000` do not fit the
15 GiB VM (the process was killed). `cluster_size 250, max_lists 8192` was still building after 19
minutes at about 10 GiB resident and was stopped; it is untested.

### Ablation (sel; 5 components, so leave-one-out and add-one-in)

Components: S = `sort_queries`; B = `block_filter` + `skip_tombstones`; C = `cache_intersections`;
R = retune (brute_max 5000, intersect_max 1,000,000, prefetch 32 vs 10000 / 200,000 / 24); BLD =
build `cluster_size 500, max_lists 4096` with target 1750 (vs 1000 / 2048 with target 2500).
Elite = all five.

| cell | QPS | recall@10 |
|---|---|---|
| control | 24,187 / 24,301 | 0.9067 |
| control + S | 26,478 | 0.9067 |
| control + B | 30,313 | 0.9067 |
| control + C | 24,621 | 0.9067 |
| control + R | 24,947 | 0.9045 |
| control + BLD | 26,041 | 0.9095 |
| elite - BLD | 39,241 | 0.9045 |
| elite - S | 36,300 | 0.9069 |
| elite - B | 36,144 | 0.9069 |
| elite - C | 42,092 | 0.9069 |
| elite - R | 40,917 | 0.9095 |
| **elite** | **46,048 / 45,601** | 0.9069 |

Read-out: S and B each help alone (+9%, +25%); C helps only together with S (+2% alone, +9.4% in the
elite) because it needs adjacent queries with the same labels; the effects are not additive (the five single gains over
control sum to 11.4k, the joint gain is 21.8k). R and BLD trade recall for speed
and are not result-preserving. Single-run spread: 24,187 vs 24,301 and 46,048 vs 45,601 (about 1%).

### Confirmation with the official protocol (all 100,000 public queries)

Unchanged harness (PR #356 head `68b8ba5d`), docker mode, `run.py --neurips23track filter
--algorithm warm --dataset yfcc-10M` (5 runs per search configuration, best kept), same VM,
runs interleaved (control, elite, elite, control). Control image: fork `8c9e575`, WARM `af8d4017`. Elite image: fork `f0b5c00`,
WARM `6091c519` (`perf/filter-yfcc`). QPS at recall@10 >= 0.90:

| run | search configuration | QPS | recall@10 |
|---|---|---|---|
| control 1 | brute_max 5000, target 2500 | 24,457 | 0.9044 |
| control 1 | brute_max 10000, target 2500 | 23,811 | 0.9067 |
| control 2 | brute_max 5000, target 2500 | 24,332 | 0.9044 |
| control 2 | brute_max 5000, target 2750 (control's best) | 24,557 | 0.9123 |
| control 2 | brute_max 10000, target 2500 | 22,182 | 0.9067 |
| elite 1 | brute_max 5000, target 1750 (chosen on sel) | 47,920 | 0.9064 |
| elite 2 | brute_max 5000, target 1750 (chosen on sel) | 45,768 | 0.9064 |
| elite 1 | brute_max 3000, target 1750 (best of its six) | 48,537 | 0.9061 |
| elite 2 | brute_max 3000, target 1750 | 47,216 | 0.9061 |
| elite 1 / 2 | brute_max 5000, target 1650 | 47,947 / 47,110 | 0.9014 |

The configuration chosen on the held-out split scores 47,920 and 45,768 on the official protocol
(spread 4.5% between two runs), 1.87-1.96 times the control's best of two runs (24,557). The sel recall of the
chosen configuration (0.9069) matches the all-queries recall (0.9064) to 0.0005, so the choice
transferred. `brute_max 3000` was not evaluated at target 1750 on sel, so it is a post-hoc
neighbour on the public set and is not the headline. The control's own run-to-run spread in this
setup is large (earlier Ice Lake docker runs of the same configuration: 22.0k and 27.8k); the
elite's two-run spread is smaller but only two runs were made. Index 9.35 GiB (harness
`indexsize` 9.80e6 KB), build 443-465 s against 226 s for the control. The 16 GiB evaluation VM has
the same memory as this clone; the base file is memory-mapped, and peak resident memory of the
elite was not measured in the docker runs.

Not covered: the Azure D8lds_v5 evaluation VM (different host, same CPU class), the private query
set, and any run-to-run spread beyond two elite runs.



## Frozen Azure confirmation, 2026-11-09–10

On Azure E8bds_v5 (Xeon Platinum 8370C, 4 cores / 8 threads), the same frozen
`6091c519` entry and headline `brute_max=5000, target=1750` measured **54,721.70
and 53,972.75 QPS**, both at recall@10 **.906415**. Each official Docker run
covered all 100,000 public queries and retained the complete submitted six-point
grid; the headline was not reselected from that grid.

The container limit was 16 GiB with no swap. Peak process RSS was 14.3841 and
14.3846 GiB; this is GNU-time RSS, not cgroup memory.peak. Complete workload wall
time was 456.23 / 456.39 seconds. The host page cache was dropped before each run.
[Raw exports, frozen protocol and archive receipt](../../benchmarks/results/azure/az1-2026-11-09/)
retain both runs and every configuration.

E8bds_v5 has 64 GiB host RAM and different storage limits from the official
D8lds_v5. These results therefore confirm the entry on an Azure Ice Lake host,
but do not establish official placement or a private-query result. They remain
below the previously reported leading filter-board throughput.

## Not done

- Graph search with a tag predicate for the common tags (filtered-Vamana / ACORN style) and
  per-list tag signatures (Bloom) to skip lists: not built; the remaining recall loss is in the
  1-tag queries with more than 100k matches (0.785 and 0.715 at target 2500).
- A 4-bit screen with 8-bit re-rank over the copied lists.
- Inserts need a rebuild; deletes are tombstones.

## Post-freeze command (Azure D8lds_v5, maintainers' procedure)

```
python install.py --neurips23track filter --algorithm warm
python run.py --neurips23track filter --algorithm warm --dataset yfcc-10M
python data_export.py --out res.csv
```

with `neurips23/filter/warm` from the fork branch `filter/warm`, whose Dockerfile pins the WARM
commit.
