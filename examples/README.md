# Examples

Every example runs on this machine with no API key, no network and no model download. Commands run from the repository root. Rust examples use the `writerslogic_warm` crate, Node ones the native addon (`npm ci --omit=optional && npm run build` once), Python ones the `warm-python` wheel or the `warm-sdk` client.

## Runnable examples

| Example | Language | What it shows | Run command | Needs |
|---------|----------|---------------|-------------|-------|
| [`quickstart/rust`](quickstart/README.md) | Rust | Ingest, search, FTS5 comparison, encrypted store, changing facts | `cargo run --release --manifest-path examples/quickstart/rust/Cargo.toml` | Rust |
| [`quickstart/node`](quickstart/README.md) | Node | The same quickstart on the native addon | `node examples/quickstart/node/quickstart.mjs` | built addon, Node 24+ |
| [`quickstart/python`](quickstart/README.md) | Python | The same quickstart over HTTP, plus masked vectors | `WARM_API_KEY=quickstart uv run --no-project --with warm-sdk python examples/quickstart/python/quickstart.py` | running `warm-server` |
| [`quickstart/js-client`](quickstart/README.md) | Node | `warm-sdk` client: remember, recall, get, forget | `WARM_URL=http://127.0.0.1:8080 WARM_API_KEY=quickstart node examples/quickstart/js-client/quickstart.mjs` | running `warm-server`, built `bindings/js/warm-sdk` |
| [`demo-offline-agent`](demo-offline-agent/README.md) | Rust | An assistant memory: batch ingest, metadata filter, replace, forget, relations, restart | `cargo run --release --example demo-offline-agent` | Rust |
| [`wasm`](wasm/README.md) | JS | Browser note search on `warm-wasm` with OPFS or IndexedDB persistence | `node examples/wasm/serve.mjs` | built `bindings/wasm/npm` |
| [`temporal-memory.cjs`](temporal-memory.cjs) | Node | Remember, replace, forget, verify across separate processes | `node examples/temporal-memory.cjs remember ./demo` (then `replace`, `forget`, `verify`) | built addon |
| [`local-documents.mjs`](local-documents.mjs) | Node | Document memory with optional local embedder and reranker | `node examples/local-documents.mjs ./demo` | built addon; model dirs optional (`WARM_EMBEDDING_DIR`, `WARM_RERANKER_DIR`) |
| [`node-observability.mjs`](node-observability.mjs) | Node | `explainQuery`, `indexStatus`, `storageHealth` | `cd examples && node node-observability.mjs` | built addon |
| [`semantic-search.mjs`](semantic-search.mjs) | Node | Ingest documents with `DocumentMemory` and search by BM25 relevance | `node examples/semantic-search.mjs` | built addon |
| [`triplet-knowledge.mjs`](triplet-knowledge.mjs) | Node | Store relational triples (`meaningEnabled: true`) and run `structuralQuery` and `multiHopQuery` | `node examples/triplet-knowledge.mjs` | built addon |
| [`sequence-encoding.mjs`](sequence-encoding.mjs) | Node | Encode ordered sequences and query with `querySequence` | `node examples/sequence-encoding.mjs` | built addon |
| [`analogy-reasoning.mjs`](analogy-reasoning.mjs) | Node | Solve `A:B :: C:?` analogies over memorized lexical hypervectors with `findAnalogy` | `node examples/analogy-reasoning.mjs` | built addon |
| [`external-embeddings.mjs`](external-embeddings.mjs) | Node | Ingest and query `Float32Array` vectors via `memorizeVector` and `queryVector` (`WARM_ENCRYPTION=off`) | `node examples/external-embeddings.mjs` | built addon |
| [`organizational_knowledge.rs`](../docs/integrations/organizational-knowledge.md) | Rust | Namespaced import, search and dry-run forget of org documents | `cargo run --release --example organizational_knowledge -- import ./store eng doc.json` | Rust |
| [`zero_trust_demo.rs`](zero_trust_demo.rs) | Rust | Client-side vector masking: ranking without seeing plaintext coordinates | `cargo run --release --features security --example zero_trust_demo` | Rust |
| [`verify_cogmem_sample.rs`](verify_cogmem_sample.rs) | Rust | Verify the cogmem C2PA sample's signed statements in [`fixtures/`](fixtures) | `cargo run --release --features provenance --example verify_cogmem_sample` | Rust |
| [`emit_w3c_credential.rs`](emit_w3c_credential.rs) | Rust | Emit a W3C credential, a revoked one and a status list for interop checks | `cargo run --release --features provenance --example emit_w3c_credential` | Rust |
| [`kmer_encoder.rs`](kmer_encoder.rs) | Rust | Encode DNA reads as sparse hypervectors and score containment | `cargo run --release --features experimental --example kmer_encoder` | Rust |
| [`vector_memory.py`](vector_memory.py) | Python | Index size by residual-bit setting of `VectorIndex` | `python examples/vector_memory.py` | `warm-python` wheel (`maturin develop --release`; on macOS 27 add `CARGO_PROFILE_RELEASE_STRIP=none`), numpy |
| [`python_phase.py`](python_phase.py) | Python | Phase vectors: bind, unbind, resonator factorization | `python examples/python_phase.py` | `warm-python` wheel |

Language bindings have their own quickstarts: [`c`](../bindings/c/README.md), [`go`](../bindings/go/README.md), [`dotnet`](../bindings/dotnet/README.md), [`java`](../bindings/java/README.md), [`swift`](../bindings/swift/README.md), [`kotlin`](../bindings/kotlin/README.md), [`wasm`](../bindings/wasm/npm/README.md), [`js`](../bindings/js/warm-sdk/README.md), [`python`](../bindings/python/warm-sdk/README.md).

## Research and benchmark examples

Measurement tooling for the committed results in `benchmarks/results/`. Not tutorials; arguments and output formats can change with the experiment.

| Example | Purpose |
|---------|---------|
| `batched_vector_search.rs` | Batched graph search through the experimental batch pool |
| `compaction_updates.rs` | Reader latency while updates and compaction run |
| `competitive_hybrid.rs` | WARM arm of the hybrid contract benchmark |
| `competitive_text.rs` | WARM arm of the competitive text benchmark |
| `competitive_text_tantivy.rs` | tantivy arm of the competitive text benchmark |
| `document_competition.rs` | Document ingest and search rows for the comparison harness |
| `document_operations.rs` | Concurrent document operations and restart consistency |
| `encoder_version_probe.rs` | Store behaviour across an encoder version change |
| `experience_ingest_bench.rs` | Ingest throughput and bytes per step of the experience core |
| `experience_lme2_eval.rs` | Gold-phrase coverage at equal token budgets on LongMemEval-V2 |
| `forget_cost.rs` | `Memory::forget` latency by erasure mode and store size |
| `history_cost.rs` | Cost of a historical view against a current-state query |
| `memory_write_bench.rs` | Per-call cost of `remember`, `remember_many` and `forget` |
| `purge_bench.rs` | Delete and search latency of the scrubbing delete |
| `qgraph_build_profile.rs` | Allocation and time profile of one vector-index build |
| `sparse_composition.rs` | Sparse-vector composition and counting-bloom payload measurements |
| `sparse_sweep.rs` | Recall and QPS sweep of the learned-sparse index |
| `supersession_probe.rs` | Supersession on the public STALE data |
| `vector_discovery_diagnosis.rs` | Diagnostic of graph construction and search on a synthetic or prepared set |
| `vector_glove_diagnosis.rs` | Plain versus profiled graph build on GloVe |
