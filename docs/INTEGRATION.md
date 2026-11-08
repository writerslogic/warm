# Integration guide

## Rust

Use `WarmCore` for vectors, documents, and structural knowledge. The default feature set is
empty; enable `security` for encryption/signing and `provenance` for provenance APIs. Optional
experimental modules have separate compatibility guarantees. Rust 1.89 or newer is required.

```rust
use writerslogic_warm::WarmCore;

fn main() -> anyhow::Result<()> {
    let memory = WarmCore::new(16384, Some("./memory".into()), None)?;
    memory.memorize("greeting".into(), memory.encode_text("hello world"))?;
    memory.flush()?;
    Ok(())
}
```

## Node.js

CommonJS and ESM both export `WarmMemorySystem`. Native operations return promises when
work is scheduled off the event loop. For documents, import `DocumentMemory` from
`warm-sdk/semantic`; it serializes model work, bounds its queue, and supports async
iterable ingestion. The default document search is lexical BM25. Add an embedder for cosine
semantic candidates and optional reranking. See [the example](../examples/local-documents.mjs)
and the README for the complete setup.

Install `@huggingface/transformers` separately to use the local ONNX factories. Supply an
existing model directory and explicit model/revision identity. The store schema must match
the adapter's embedding space. Keep models alive while operations are pending and dispose
of them after ingestion/search finishes. No model is downloaded by these factories.

`memorizeFile` now ingests a bounded UTF-8 file as chunks. Use `searchDocuments` to retrieve
passages and `deleteDocument` to remove it. The earlier whole-file vector behavior can be
expressed explicitly with `memorizeText` for bounded inputs if needed.

## Python

The Python wheel exposes `PhaseHVec` and `PhaseResonator`, not the persistent document/search
engine. Build with `maturin develop --features python`:

```python
from writerslogic_warm import PhaseHVec

a = PhaseHVec.random(1024, 8, 1)
b = PhaseHVec.random(1024, 8, 2)
assert -1.0 <= a.similarity(b) <= 1.0
```

### Streaming partition index (experimental)

`PartitionIndex` is a streaming, updatable L2 index. Before `train_at` vectors are live, searches are exact brute force. At the first search after that point, it trains an IVF with SOAR spill and PQ residual codes, then answers queries with a 4-bit LUT scan followed by an exact f32 reorder. It is the index behind the big-ann-benchmarks streaming entry ([BIG-ANN-STREAMING](benchmarks/BIG-ANN-STREAMING.md)).

The published wheels do not include it. Build from source with `maturin develop --features python,experimental`:

```python
import numpy as np
from writerslogic_warm import PartitionIndex

idx = PartitionIndex.create(100, lists=5000, train_at=1_000_000, nprobe=250,
                            reorder=317, pq_m=50, threads=0, seed=7)
idx.insert(np.arange(n, dtype=np.int64), x)      # x: (n, 100) float32; ids fit in u32
idx.delete(np.array([3, 7], dtype=np.int64))      # returns the number removed
ids = idx.search(q, 10)                           # (nq, 10) int64, padded with -1
idx.set_nprobe(300)
print(len(idx), idx.stats())
```

Notes:
- `k` is at most 4,096.
- The query width must equal `dim`.
- `threads=0` uses all available cores.
- The centres are fixed once trained. Later inserts are assigned to the existing lists.

## Portability

Use the Rust or native Node APIs for interoperable encodings. The current text encoder is
versioned as `multiscale-words-v1`; a generic character-trigram implementation does not produce
compatible vectors. Dense projection version 2 preserves sign, and old vectors must be
re-encoded from their original inputs. See [migration](production-readiness.md).

The persistent native engine depends on file locking and mmap. A separate
[WebAssembly binding](runtimes/wasm/README.md) runs the document store in browsers and other
JavaScript runtimes with snapshot persistence; it is available from source and is not yet
published on npm. It does not include the graph or knowledge-graph engine. Applications in other
languages can expose their own service around the native engine with authentication and limits
appropriate to their use.
