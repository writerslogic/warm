---
template: home.html
title: WARM
description: Hybrid search and agent memory that runs on your machine, with no service calls.
hide:
  - navigation
  - toc
  - path
---

## What WARM is

WARM (WritersLogic Adaptive Retention & Memory) is a Rust engine for local agent and application
memory. It provides stemmed BM25 and exact-cosine hybrid document search, a graph vector index, a
memory API (`remember`, `recall`, `forget`, `explain`) and a vector-symbolic fact store. A store is a
directory on your machine; the library makes no network calls.

Use it when you want documents or agent memory searchable inside your own process, over HTTP or from
an MCP client, with storage you control. Look elsewhere if you need a distributed, multi-writer
service: a store has one writer, and `warm-server` is a single node.

Document search is conventional hybrid retrieval. The holographic part is the vector-symbolic layer;
[concepts](concepts.md) says which is which. Every number on this site traces to a committed file
under [`benchmarks/results/`](https://github.com/writerslogic/warm/tree/main/benchmarks/results),
and the [benchmarks page](benchmarks/index.md) lists where WARM loses.

| I want to | Go to |
|---|---|
| Install and run a first query | [Quickstart](quickstart.md) |
| Understand the model | [Concepts](concepts.md) |
| Look up an API or feature flag | [API overview](reference/api.md) |
| Connect a framework, editor or language | [Integrations](integrations.md) |
| Run the HTTP service | [Server](server/index.md) |
| Read or verify a store on disk | [File format](file-format/index.md) |
| Check performance claims | [Benchmarks](benchmarks/index.md) |
| Check security and stability promises | [Security](security.md) |
| Common questions | [FAQ](reference/faq.md) |
| Release notes | [Changelog](reference/changelog.md) |
