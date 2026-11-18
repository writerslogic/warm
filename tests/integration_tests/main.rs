// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Integration tests that run in one process. Tests that re-execute the test binary, set
//! process-global state (environment, working directory, global allocator) or arm failpoints
//! stay in their own `tests/*.rs` targets.

#[path = "../format_common/mod.rs"]
mod format_common;
#[cfg(feature = "server")]
mod server_support;

mod support;

mod chunking_proptest;
#[cfg(all(feature = "security", unix))]
mod committed_sidecars;
#[cfg(feature = "conversation-memory")]
mod conversation_context;
#[cfg(feature = "conversation-memory")]
mod conversation_memory;
#[cfg(all(feature = "conversation-memory", feature = "local-models"))]
mod conversation_models;
mod document_checkpoint;
mod document_chunks;
mod document_competition;
mod document_dense;
mod documents;
#[cfg(feature = "local-embed")]
mod embed_store;
#[cfg(feature = "security")]
mod enc_text;
mod experience_default;
mod experience_form_state;
mod forget_equivalence;
mod format_crash;
mod format_layout;
mod format_upgrade_regressions;
mod ingest_encode;
mod ingest_postings;
mod legacy_analyzer_fixture;
mod lexical_count;
mod matryoshka_vectors;
#[cfg(feature = "server")]
mod memory_api_server;
mod native_memory_workflow;
mod qgraph_prefetch;
#[cfg(feature = "experimental")]
mod quantized_ivf;
mod reliability;
mod rss_embeddings;
#[cfg(feature = "provenance")]
mod scitt_receipt_interop;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
mod server_abuse;
#[cfg(feature = "server")]
mod server_ann;
#[cfg(feature = "server")]
mod server_api;
#[cfg(feature = "server")]
mod server_ops;
mod simulation;
mod sparse_documents;
mod store_lifecycle;
mod time_travel;
#[cfg(feature = "security")]
mod wrong_passphrase;
