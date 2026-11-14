<a href="https://writerslogic.com/warm/"><img src="https://raw.githubusercontent.com/writerslogic/warm/main/assets/crystal-halo/crystal-detailed-static.png" alt="WARM logo" width="96" align="left"></a>

# WARM for Python

Native Python bindings for WARM's vector search and vector-symbolic algebra.

[Website](https://writerslogic.com/warm/) · [Documentation](https://writerslogic.com/warm/docs/) · [Source](https://github.com/writerslogic/warm) · [License](https://github.com/writerslogic/warm/blob/main/LICENSE)

<br clear="left">

## Install

Python 3.9 or newer is required. Install the native binding; NumPy (`>=1.26`, which the vector-index API needs) is installed with it:

```sh
uv add warm-python
```

The distribution is named `warm-python`; the import is `writerslogic_warm`.

For the pinned default embedding model with no first-use download, install
`warm-python[embed]`. That extra adds the separate
[`warm-models`](https://pypi.org/project/warm-models/) wheel.

## Vector search

`VectorIndex` builds an approximate nearest-neighbor index over vectors you supply. It supports
angular, Euclidean, and inner-product search. The returned IDs are row numbers in the input matrix.

```python
import numpy as np
from writerslogic_warm import VectorIndex

vectors = np.random.default_rng(7).normal(size=(128, 32)).astype(np.float32)
index = VectorIndex("angular", degree=16, build_ef=64)
index.fit(vectors)
index.set_query_arguments(ef=64, rerank=32)
matches = index.query(vectors[0], 5)
print(matches.tolist())
```

The binding also provides `UpdatableIndex` for inserts and deletes, `FilteredIndex` for restricted
searches, and `SparseIndex` for sparse vectors.

`index.save(path)` persists a `VectorIndex`; `index.load(path)` reads it into private memory.
Use the same metric and `direct_ip` setting when loading. On Linux, source builds after
`4299b539` also support `index.load(path, shared=True)` for an immutable memfd snapshot.
The loader requires `F_SEAL_WRITE`, `F_SEAL_SHRINK` and `F_SEAL_GROW`; ordinary files and
`F_SEAL_FUTURE_WRITE` alone are rejected. Each worker must open the **same snapshot inode**
to share physical pages. A loaded mapping remains valid after its publishing descriptor closes.
The normal private-memory load remains the default on every platform.

[`sealed_index_run.py`](../../scripts/lowlevel/sealed_index_run.py) shows how a publisher copies
an index once, seals it, keeps it available while worker processes run, and records the snapshot
hash and setup time. This option is not in the published `warm-python==0.6.4` wheel used by the
VectorDBBench entry.

With the `[embed]` extra, the pinned default text embedding model is available offline:

```python
from writerslogic_warm import Embedder

embedder = Embedder(offline=True)
print(len(embedder.embed_query("Where is my note?")))
```

## Phase-vector algebra

`PhaseHVec` supports deterministic phase-vector binding, unbinding, bundling, and similarity.

```python
from writerslogic_warm import PhaseHVec

key = PhaseHVec.random(1024, 64, seed=1)
value = PhaseHVec.random(1024, 64, seed=2)
stored = key.bind(value)
assert stored.unbind(key).phases() == value.phases()
```

`resonator_factorize` searches codebooks for factors of a bound phase vector.

Phase vectors in bundles and factor codebooks must match the composite's dimension and
resolution. Invalid shapes and resource limits raise `ValueError`. Empty factor codebooks
are rejected; an empty list of factors returns `[]`. Zero dimensions and zero iterations
retain the Rust API's behavior.

The Python boundary admits at most 65,536 dimensions, 65,536 input vectors, and 4,194,304
total phase elements per operation. Resonators additionally allow at most 64 factors,
1,000 iterations, and 100,000,000 estimated component visits, computed as
`(3 * entries + factors**2) * max(dimensions, 1) * (iterations + 1)` before cloning codebooks
or allocating the resonator index.

## Document search and memory

`Warm` is the in-process engine: a persistent store with hybrid BM25 and exact-cosine document
search, the memory API and vector search. Requests and results are dicts and lists with the
camelCase fields of the Node binding. One process may write a store at a time.

```python
from writerslogic_warm import Warm

store = Warm(16384, "./warm-data")
store.memorize_document({"id": "a", "text": "The harbor opens at six in the morning."})
print(store.search_documents("when does the harbor open", {"k": 3})[0]["documentId"])

store.remember({"text": "Ada prefers tea", "subject": "ada"})
print(store.recall("what does Ada drink")["results"][0]["text"])
```

`memorize_vector(id, vector)` and `query_vector(vector, k)` use the vertex graph index;
`query_vector` returns `(id, cosine)` pairs.

## Choosing the right Python package

This native binding runs the engine in your Python process: document search, the memory API,
vector search and phase-vector operations. For an HTTP client to `warm-server`, use
[`warm-sdk`](https://pypi.org/project/warm-sdk/).
