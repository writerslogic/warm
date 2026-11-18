---
template: home.html
title: WARM
description: Hybrid search, graph indexing, and vector-symbolic memory. Local-first retrieval and associative memory with crash-safe storage and no service calls.
hide:
  - navigation
  - toc
  - path
---

## How to read this site

The document pipeline is conventional retrieval: stemmed BM25 fused with exact cosine over
embeddings you supply. The vector-symbolic layer powers the meaning memory, composition and the
experimental vector index. Every number on this site traces to a committed file under
[`benchmarks/results/`](https://github.com/writerslogic/warm/tree/main/benchmarks/results), and
the [benchmark pages](PUBLIC-BENCHMARKS.md) say where WARM loses to another system.

| I want to | Go to |
|---|---|
| Install | [Install](get-started/install.md) |
| Run a first query | [Quickstart](quickstart/index.md) |
| Look up an API | [API reference](api/index.md) |
| Pick a runtime | [Choose an integration](get-started/choose.md) |
| Understand "holographic" | [Concepts](concepts/holographic.md) |
| Look up a term | [Glossary](reference/glossary.md), [FAQ](reference/faq.md) |
| Check standards claims | [Standards and provenance](reference/standards.md) |

The version selector switches between `latest` (the newest tagged release that carries these
docs), `dev` (`main`) and each tag. Build and checks: [docs pipeline](community/docs-pipeline.md).
