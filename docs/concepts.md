# Concepts

## Documents and search

A document has an ID, text, an optional source URI and metadata, and optionally one dense
embedding per chunk that you supply. Text is split into chunks (256 words by default) and written
in one atomic batch. A store is a directory; one process writes it at a time and a second writer
fails at open with a lock error. Replacing or deleting a document is atomic.

Search is conventional hybrid retrieval:

- **Lexical:** stemmed BM25 over the chunk terms. It supports `+required`, `-excluded` and
  quoted phrases (flat queries, no nesting). An empty or only-excluded query matches nothing.
- **Dense:** exact cosine over the embeddings you supplied, scanning all eligible chunks.
- **Hybrid:** the two ranked lists are fused, either by a min-max normalised blend or by
  reciprocal rank fusion.
- Hits carry the passage text, its byte range in the source and the score. A metadata filter
  narrows the eligible chunks.

No embedding model is needed for lexical search. WARM does not generate embeddings unless you
enable `local-embed` or `local-models`.

The document pipeline does not rank with hypervectors. Each chunk carries a sparse hypervector
that is serialised with it, but `src/core/documents/search.rs` does not read it.

## Memory API

`remember`, `recall`, `forget` and `explain` sit on top of the document engine. A fact is an
ordinary document, so document search, deletion and the raw index keep working on a store the
memory API wrote.

| Call | Behavior |
|---|---|
| `remember` | Stores a fact with an optional `subject`, `validFrom` and `supersedes` list. A policy such as `latest-by-subject` can supersede automatically. Observations that are trees are routed to the experience core. |
| `recall` | Returns current facts; `asOf` returns what held at that time. Superseded facts are hidden, not destroyed. |
| `explain` | Says why a given fact was or was not returned for a query. |
| `forget` | Removes facts by subject, ID or scope. It removes them from the store, not from what a model inferred. |

Forgetting is not secure erasure: removed records can remain in obsolete segments, snapshots and
backups until compaction and backup rotation remove them.

## Experience core

For observations that are trees and that repeat, such as the accessibility tree an agent sees after
each action. Nodes are hash-consed, so a subtree seen in many episodes is stored once. Trees are
cut into regions, and per-template effect tables record what each action changed. Purging an
episode frees everything no other episode holds. On a synthetic repeating workload the snapshot
shrank to 0.022 of the raw tree text at 1,000 episodes
([`experience_ingest_synthetic.json`](https://github.com/writerslogic/warm/blob/main/benchmarks/results/experience_ingest_synthetic.json));
that ratio depends on how much the pages repeat and was not run beyond 1,000 episodes.

## Fact store and the holographic layer

The vector-symbolic layer is the part that is holographic. A hypervector is a sparse binary
vector of `D` dimensions with about `D/256` active bits by default; atoms get vectors from a
hash seed, so no codebook is stored.

| Operation | Meaning |
|---|---|
| Bind | Symmetric difference of the active sets. Ties a role to a filler and is its own inverse. |
| Bundle | Per-index count above a threshold. Superposes many items into one vector similar to each. |
| Permute | Cyclic shift. Encodes order. |

The meaning memory stores `(subject, relation, object)` triples as composite vectors. A query
binds the known parts, and a Hopfield cleanup snaps the noisy result to the nearest stored atom;
`multi_hop` chains relations. It is a runtime opt-in (`MeaningConfig::enabled`). The sparse vector
API (`memorize_vector`, `query_vector`) is backed by an updatable vertex graph.

Measured limits, all over the tested range only:

- A union bundle at D = 16,384 and density 1/256 keeps a positive member gap to N = 500 (4% false
  positives) and was tested to N = 2,500. Other dimensions and densities are in
  [`benchmark_scaling_results.json`](https://github.com/writerslogic/warm/blob/main/benchmarks/results/benchmark_scaling_results.json).
  A single point is not a guarantee, and nothing is claimed past the largest N tested.
- With 100 stored vectors and 30 probes, nearest-neighbour and Hopfield cleanup were correct at
  every corruption level from 0 to 90 percent, in the same file. This does not cover larger
  codebooks, superposed probes or adversarial noise.

## Storage and encryption

A store is a directory with a journal, segment files, a lexical index, optional vector index caches
and a `store.json` manifest. Writes are journaled and crash-safe, compaction is verified, and a
process lock allows one writer. The layout is in [file format](file-format/index.md).

With the `security` feature a new store is encrypted with AES-256-GCM by default. Encryption
covers arena payloads, persisted index caches and the passage text segment. It does not cover
file names, sizes, topology or salts. Details and the opt-out are in
[security](security.md#encryption-at-rest).

Sketches and term counts are not encryption: the encoder is public and deterministic, so anyone
holding a store can test candidate texts. `storeText: false` keeps term counts, IDs and
supplied embeddings. Treat embeddings as sensitive as the text.
