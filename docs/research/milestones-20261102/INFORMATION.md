# Information-navigation falsifier

The first frozen proxy family is rejected. Access to more unseen or low-indegree
neighbors did not reliably identify useful bridges better than distance alone.
All three candidate policies failed the preregistered promotion rule. No serving
policy was changed and no held-out inputs were used.

Each entry below counts additional exact-oracle neighbors encountered over 1,000
queries on the exposed historical 100K GloVe graph. These are candidate-coverage
ceilings, not final reranked recall. All methods share a 256-vertex shadow, a
32-expansion cap and the stated additional-vector budget.

| ef | Vector budget | Distance control | Unseen count | Unseen rarity | Alternating distance/rarity |
|---|---|---|---|---|---|
| 128 | 256 | 58 | 34 | 37 | 49 |
| 128 | 512 | 98 | 59 | 74 | 85 |
| 256 | 256 | 12 | 17 | 17 | 11 |
| 256 | 512 | 25 | 29 | 35 | 28 |

The apparent 40% recovery gain for rarity at ef=256/budget=512 is only ten extra
neighbors among 10,000 oracle positions. The same method loses 24 recovered
neighbors at ef=128/budget=512 and lowers worst candidate coverage from 20% to
10%. It also performs about 87,019 priority-neighbor visits per query at the former
setting versus 31 for distance, on top of the common adjacency cache. Equal
vector budgets are not equal compute. Selectively reporting the positive cell
would misrepresent this result.

The completed experiment covers both 10K and 100K graphs, both ef values, both
budgets and all four policies: 32 cells, each with 1,000 query observations. The
same queries are reused across policies and budgets; these are paired outcomes,
not independent samples. All 10K results, including wins and losses, are retained
in [information-finished.json](information-finished.json). Selection never receives
the oracle or original answer IDs. Four regression tests cover selection,
overlap, caps, deterministic ties and bounded input streaming.

Inference: this rejects these particular degree-based proxies and the fixed
one-hop extension. It does not establish that navigation value equals distance,
that topology is optimal, or that all information-directed search is ineffective.
Relevance remains necessary: an unseen region can be large and still contain
no answers. Do not respond by tuning this proxy family on the same results or
shipping the best isolated cell.

## Reproduction and failed attempts

```sh
node --test benchmarks/research/vector_topology/information-test.mjs
node benchmarks/research/vector_topology/information-run.mjs /absolute/new/output
```

The [frozen protocol](../../../benchmarks/research/vector_topology/INFORMATION-PLAN.json)
specifies inputs, policies, budgets and the promotion rule. The completed source
snapshot, hashes and all selected/newly encountered IDs are under
`/Volumes/A/.hms-bench-cache/warm-information-navigation-20261102-v3/`.
The input hashes tie the run to the complete milestone-1 traces. Its
[freeze](information-freeze.json) and [summary](information-finished.json) are
copied here. The run stayed below the original 2 GiB RSS and 900-second limits.

Attempts v1 and v2 remain beside v3. V1's controller incorrectly expected a
`corpusRows` field in the older 100K receipt; the absent size propagated into
labels and a decision branch. Its result is invalid. The correction explicitly
declares input sizes and checks decoded topology lengths; it changes no policy,
budget, threshold or query. V2 hit the unchanged 2 GiB RSS limit while its
line reader queued input ahead of asynchronous result writes. V3 uses a bounded,
backpressured byte parser with a 64 MiB line ceiling. It completes all cells.
The failed runs are not treated as qualified evidence.
