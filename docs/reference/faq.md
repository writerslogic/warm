---
description: Answers to the questions people ask about WARM, each traced to a documentation page, the code or an open issue.
---

# FAQ

Each answer links its source. Not here? Open an [issue](https://github.com/writerslogic/warm/issues) or ask in [Discussions](https://github.com/writerslogic/warm/discussions).

## Is the document search actually holographic?

No. Document search chunks text, scores it with stemmed BM25 and exact cosine over embeddings
you supply, and fuses the two lists. It does not rank with hypervectors. The holographic part
is the vector-symbolic layer: the meaning memory, composition, sequences and the experimental
vector index. [What "holographic" means here](../concepts.md) lists which is which
and shows how to check it in the code.

## Do I need embeddings?

No. Text search works from the text alone, with stemmed BM25. Supply an embedding per chunk and
WARM also scores exact cosine and blends the two, which is where it can beat a lexical engine on
paraphrases. The [quickstart](../quickstart.md) runs both ways.

## Why does the SDK need a server?

`warm-sdk` is an HTTP client; the server owns the engine and one store per tenant. To run the
engine in-process, use `writerslogic-warm` (Node) or `warm-python` (`writerslogic_warm.Warm`:
document search, the memory API and vector search, one writer per store). See
[integrations](../integrations.md) and the [server page](../server/index.md).

## Does it send anything over the network?

The library makes no network calls and sends no telemetry. The HTTP clients send requests only
to the server URL you configure; they do not start a server or download models. Two opt-in Cargo
features touch a network: `server` listens for HTTP requests, and `provenance-scitt` submits
signed statements to a transparency service you configure. Neither is on by default
(`default = []`). A remote deployment needs the controls in the [server guide](../server/index.md).

## Is my data encrypted on disk?

With the `security` feature compiled in, a new store is encrypted by default (AES-256-GCM) and
plaintext is an explicit opt-out. Without `security` it is plaintext. What encryption covers, and
what a stored hypervector still reveals, is on [security](../security.md#encryption-at-rest);
read it before relying on encryption at rest.

## Opening an encrypted store with the wrong passphrase says "cannot replay arena frame at 0"

That is the current error: the real cause, an authenticated-decryption failure, is wrapped in
the replay context. It means the passphrase is wrong or the store is damaged. A clearer message
is tracked in [issue 91](https://github.com/writerslogic/warm/issues/91).

## How many items fit in one bundle?

There is no single safe number. The capacity depends on the dimension and density, and the
false-positive rate climbs well before the bundle saturates. The
[concepts page](../concepts.md#fact-store-and-the-holographic-layer) reports what was swept, where the sweep stops, and that
earlier "hard wall" figures were wrong and are retired.

## Where does WARM lose?

The benchmark pages say so next to the wins. On vector search the sparse vector path is far
below exact search on recall, and the quantized graph index does not reach 0.95 recall@10 on
nytimes-256 where FAISS, hnswlib, Lucene and RaBitQ do. On SciFact the hybrid search is the
best WARM configuration. Start at [Public benchmarks](../benchmarks/index.md).

## Which Rust or Node version do I need? (MSRV)

The MSRV (minimum supported Rust version) is Rust 1.89; Node 18 or newer for the bindings. The `local-models` feature and `--all-features` need Rust 1.94.

## Why is `npm run build` changing `index.d.ts`?

The napi build regenerates it, and the file is both tracked and listed in `.gitignore`. Do not
commit that diff by accident. The cleanup is tracked in
[issue 104](https://github.com/writerslogic/warm/issues/104).

## Which license is it under?

Apache-2.0. There is no CLA; you certify your right to contribute with a sign-off on each
commit. See [CONTRIBUTING.md](https://github.com/writerslogic/warm/blob/main/CONTRIBUTING.md).
