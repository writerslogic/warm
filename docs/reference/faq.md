---
description: Answers to the questions people ask about WARM, each traced to a documentation page, the code or an open issue.
---

# FAQ

Each answer links its source. Not here? Open an [issue](https://github.com/writerslogic/warm/issues) or ask in [Discussions](https://github.com/writerslogic/warm/discussions).

## Is the document search actually holographic?

No. Document search chunks text, scores it with stemmed BM25 and exact cosine over embeddings
you supply, and fuses the two lists. It does not rank with hypervectors. The holographic part
is the vector-symbolic layer: the meaning memory, composition, sequences and the experimental
vector index. [What "holographic" means here](../concepts/holographic.md) lists which is which
and shows how to check it in the code.

## Do I need embeddings?

No. Text search works from the text alone, with stemmed BM25. Supply an embedding per chunk and
WARM also scores exact cosine and blends the two, which is where it can beat a lexical engine on
paraphrases. The [quickstart](../quickstart/index.md) runs both ways.

## Why does the SDK need a server?

`warm-sdk` is an HTTP client; the server owns the engine and one store per tenant. To run the
engine in-process, use `writerslogic-warm` (Node) or `warm-python` (`writerslogic_warm.Warm`:
document search, the memory API and vector search, one writer per store). See
[which package](../get-started/choose.md#which-package) and the [server page](../SERVER.md).

## Does it send anything over the network?

The library makes no network calls and sends no telemetry. The HTTP clients send requests only
to the server URL you configure; they do not start a server or download models. Two opt-in Cargo
features touch a network: `server` listens for HTTP requests, and `provenance-scitt` submits
signed statements to a transparency service you configure. Neither is on by default
(`default = []`). A remote deployment needs the controls in the [server guide](../SERVER.md).

## Is my data encrypted on disk?

Only if you turn it on. The `security` feature adds AES-256-GCM with an Argon2id-derived key.
What the encryption covers, and what a stored hypervector still reveals, is on
[Privacy limits](../concepts/privacy-limits.md); read it before relying on encryption at rest.

## Opening an encrypted store with the wrong passphrase says "cannot replay arena frame at 0"

That is the current error: the real cause, an authenticated-decryption failure, is wrapped in
the replay context. It means the passphrase is wrong or the store is damaged. A clearer message
is tracked in [issue 91](https://github.com/writerslogic/warm/issues/91).

## How many items fit in one bundle?

There is no single safe number. The capacity depends on the dimension and density, and the
false-positive rate climbs well before the bundle saturates. The
[capacity page](../concepts/capacity.md) reports what was swept, where the sweep stops, and that
earlier "hard wall" figures were wrong and are retired.

## Where does WARM lose?

The benchmark pages say so next to the wins. On vector search the sparse vector path is far
below exact search on recall, and the quantized graph index does not reach 0.95 recall@10 on
nytimes-256 where FAISS, hnswlib, Lucene and RaBitQ do. On SciFact the hybrid search is the
best WARM configuration. Start at [Public benchmarks](../PUBLIC-BENCHMARKS.md).

## Which Rust or Node version do I need? (MSRV)

The MSRV (minimum supported Rust version) is Rust 1.89; Node 18 or newer for the bindings. See the
[requirements table](../get-started/install.md#requirements) for every MSRV and runtime requirement.

## Why is `npm run build` changing `index.d.ts`?

The napi build regenerates it, and the file is both tracked and listed in `.gitignore`. Do not
commit that diff by accident. The cleanup is tracked in
[issue 104](https://github.com/writerslogic/warm/issues/104).

## Why do the docs have three versions?

`latest` is the newest tagged release that carries these docs, `dev` is `main`, and each release
tag keeps its own copy; an old version shows a banner. See
[how the docs are built](../community/docs-pipeline.md).

## Which license is it under?

Apache-2.0. There is no CLA; you certify your right to contribute with a sign-off on each
commit. See [Contributing](../community/index.md).
