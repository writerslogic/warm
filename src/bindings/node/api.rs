// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core;
use napi_derive::napi;

fn governance_report(report: core::cognition::governor::GovernanceReport) -> GovernanceReportJs {
    GovernanceReportJs {
        composites_merged: report.composites_merged as u32,
        composites_forgotten: report.composites_forgotten as u32,
        atoms_forgotten: report.atoms_forgotten as u32,
        idf_refreshed: report.idf_refreshed,
        atoms_refined: report.refinement.atoms_refined as u32,
    }
}

#[napi(object, js_name = "VectorRecordOperationInput")]
pub struct VectorRecordOperationJs {
    pub op: String,
    pub record: Option<core::vector_records::VectorRecordInput>,
    pub id: Option<String>,
}

#[napi(object, js_name = "VectorRecordQueryResult")]
pub struct VectorRecordQueryResultJs {
    pub revision: String,
    pub results: Vec<core::vector_records::VectorRecordHit>,
    #[napi(
        ts_type = "{ route: string; candidateDimensions: number; shortlistLimit?: number | null; graphDegree?: number | null; graphBuildEf?: number | null }"
    )]
    pub diagnostics: serde_json::Value,
}

#[napi]
// codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
pub struct WarmMemorySystem {
    /// `None` once closed. Calls clone the `Arc` out and run on the clone, so `close` waits
    /// for them before it releases the store lock.
    slot: crate::core::sync::RwLock<Option<Open>>,
    reopen: Reopen,
}

/// What `refresh` needs to open the store again.
struct Reopen {
    read_only: bool,
    dimensions: u32,
    storage_path: Option<String>,
    config: Option<crate::core::WarmConfig>,
    memory_config: crate::core::sync::Mutex<Option<crate::core::memory_api::MemoryConfig>>,
}

impl Reopen {
    /// A memory API over `core` with the configuration the caller last set.
    fn memory_for(&self, core: Arc<WarmCore>) -> Result<crate::core::memory_api::Memory> {
        match self.memory_config.lock().clone() {
            Some(config) => {
                crate::core::memory_api::Memory::with_config(core, config).map_err(napi_err)
            }
            None => Ok(crate::core::memory_api::Memory::new(core)),
        }
    }
}

struct Open {
    core: Arc<WarmCore>,
    memory: Arc<crate::core::memory_api::Memory>,
}

const CLOSED: &str = "[store_closed] the store is closed";

impl WarmMemorySystem {
    fn core(&self) -> Result<Arc<WarmCore>> {
        match &*self.slot.read() {
            Some(open) => Ok(open.core.clone()),
            None => Err(napi::Error::from_reason(CLOSED)),
        }
    }

    fn memory(&self) -> Result<Arc<crate::core::memory_api::Memory>> {
        match &*self.slot.read() {
            Some(open) => Ok(open.memory.clone()),
            None => Err(napi::Error::from_reason(CLOSED)),
        }
    }

    fn opened(
        core: WarmCore,
        read_only: bool,
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<crate::core::WarmConfig>,
    ) -> Self {
        let core = Arc::new(core);
        Self {
            slot: crate::core::sync::RwLock::new(Some(Open {
                memory: Arc::new(crate::core::memory_api::Memory::new(core.clone())),
                core,
            })),
            reopen: Reopen {
                read_only,
                dimensions,
                storage_path,
                config,
                memory_config: Default::default(),
            },
        }
    }
}

#[napi]
// codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
impl WarmMemorySystem {
    #[napi(
        ts_args_type = "operations: Array<{ op: 'insert' | 'upsert'; record: VectorRecordInput } | { op: 'delete'; id: string }>, options?: VectorRecordRevisionOptions",
        ts_return_type = "Promise<VectorRecordMutationResult>"
    )]
    pub async fn mutate_vector_records(
        &self,
        operations: Vec<VectorRecordOperationJs>,
        options: Option<core::vector_records::VectorRecordRevisionOptions>,
    ) -> Result<serde_json::Value> {
        let core = self.core()?;
        run_async(move || {
            use core::vector_records::VectorRecordOperation;
            let operations = operations.into_iter().map(|operation| {
                match (operation.op.as_str(), operation.record, operation.id) {
                    ("insert", Some(record), None) => Ok(VectorRecordOperation::Insert { record }),
                    ("upsert", Some(record), None) => Ok(VectorRecordOperation::Upsert { record }),
                    ("delete", None, Some(id)) => Ok(VectorRecordOperation::Delete { id }),
                    _ => anyhow::bail!("vector operation requires insert/upsert with only record, or delete with only id"),
                }
            }).collect::<anyhow::Result<Vec<_>>>()?;
            Ok(serde_json::to_value(core.mutate_vector_records(
                operations,
                &options.unwrap_or_default(),
            )?)?)
        })
        .await
    }

    #[napi(ts_return_type = "Promise<VectorRecordGetResult>")]
    pub async fn get_vector_record(
        &self,
        id: String,
        options: Option<core::vector_records::VectorRecordRevisionOptions>,
    ) -> Result<serde_json::Value> {
        let core = self.core()?;
        run_async(move || {
            Ok(serde_json::to_value(
                core.get_vector_record(&id, &options.unwrap_or_default())?,
            )?)
        })
        .await
    }

    #[napi(ts_return_type = "Promise<VectorRecordListResult>")]
    pub async fn list_vector_records(
        &self,
        options: Option<core::vector_records::VectorRecordListOptions>,
    ) -> Result<core::vector_records::VectorRecordListResult> {
        let core = self.core()?;
        run_async(move || core.list_vector_records(&options.unwrap_or_default())).await
    }

    #[napi(ts_return_type = "Promise<VectorRecordQueryResult>")]
    pub async fn query_vector_records(
        &self,
        vector: Vec<f64>,
        k: f64,
        options: Option<core::vector_records::VectorRecordQueryOptions>,
    ) -> Result<VectorRecordQueryResultJs> {
        let core = self.core()?;
        run_async(move || {
            anyhow::ensure!(
                k.is_finite()
                    && k.fract() == 0.0
                    && (1.0..=core::documents::MAX_K as f64).contains(&k),
                "k must be an integer in 1..=1000"
            );
            let result =
                core.query_vector_records(&vector, k as usize, &options.unwrap_or_default())?;
            Ok(VectorRecordQueryResultJs {
                revision: result.revision,
                results: result.results,
                diagnostics: serde_json::to_value(result.diagnostics)?,
            })
        })
        .await
    }

    #[napi(
        ts_args_type = "vector: number[], k: number, filter: Record<string, string | number | boolean>, options?: VectorRecordQueryOptions",
        ts_return_type = "Promise<VectorRecordQueryResult>"
    )]
    pub async fn query_vectra_records(
        &self,
        vector: Vec<f64>,
        k: f64,
        filter: serde_json::Value,
        options: Option<core::vector_records::VectorRecordQueryOptions>,
    ) -> Result<VectorRecordQueryResultJs> {
        let core = self.core()?;
        run_async(move || {
            anyhow::ensure!(
                k.is_finite()
                    && k.fract() == 0.0
                    && (1.0..=core::documents::MAX_K as f64).contains(&k),
                "k must be an integer in 1..=1000"
            );
            let result = core.query_vectra_records(
                &vector,
                k as usize,
                &filter,
                &options.unwrap_or_default(),
            )?;
            Ok(VectorRecordQueryResultJs {
                revision: result.revision,
                results: result.results,
                diagnostics: serde_json::to_value(result.diagnostics)?,
            })
        })
        .await
    }

    #[napi(constructor)]
    pub fn new(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfigJs>,
    ) -> Result<Self> {
        let cfg = config
            .map(|c| c.into_config())
            .transpose()
            .map_err(napi_err)?;
        let core =
            WarmCore::new(dimensions, storage_path.clone(), cfg.clone()).map_err(napi_err)?;
        Ok(Self::opened(core, false, dimensions, storage_path, cfg))
    }

    /// Opens an existing store without the writer lock, so another process can hold it for
    /// writing. Reads see the journal up to its last complete frame; `refresh()` moves to a
    /// newer one. Every mutation fails with code `read_only`.
    #[napi(factory)]
    pub fn open_read_only(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfigJs>,
    ) -> Result<Self> {
        let cfg = config
            .map(|c| c.into_config())
            .transpose()
            .map_err(napi_err)?;
        let core = WarmCore::open_read_only(dimensions, storage_path.clone(), cfg.clone())
            .map_err(napi_err)?;
        Ok(Self::opened(core, true, dimensions, storage_path, cfg))
    }

    /// True for a store opened with `openReadOnly`.
    #[napi(getter)]
    pub fn read_only(&self) -> bool {
        self.reopen.read_only
    }

    /// True after `close()`.
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        self.slot.read().is_none()
    }

    /// Moves a read-only store to the writer's latest commit, reading only what was appended
    /// since (a compaction is read once, afresh). Returns false when nothing was committed.
    #[napi]
    pub fn refresh(&self) -> Result<bool> {
        if !self.reopen.read_only {
            return Err(napi::Error::from_reason(
                "refresh applies to a store opened read-only",
            ));
        }
        let current = self.core()?;
        match current.refresh_in_place().map_err(napi_err)? {
            crate::core::Refresh::Unchanged => return Ok(false),
            crate::core::Refresh::Advanced => return Ok(true),
            // A compaction replaced the generation this store reads: open it afresh.
            crate::core::Refresh::Reopen => {}
        }
        let core = Arc::new(
            WarmCore::open_read_only(
                self.reopen.dimensions,
                self.reopen.storage_path.clone(),
                self.reopen.config.clone(),
            )
            .map_err(napi_err)?,
        );
        let memory = self.reopen.memory_for(core.clone())?;
        match &mut *self.slot.write() {
            Some(open) => {
                *open = Open {
                    core,
                    memory: Arc::new(memory),
                };
                Ok(true)
            }
            None => Err(napi::Error::from_reason(CLOSED)),
        }
    }

    /// Flushes and releases the store lock so another process can open it for writing. Safe
    /// to call twice. Every later call fails with code `store_closed`. Waits up to 10 seconds
    /// for calls in flight; if they do not finish the store stays open and this throws.
    #[napi]
    pub fn close(&self) -> Result<()> {
        // Release the slot's lock before waiting, so calls that arrive meanwhile fail with
        // `store_closed` at once instead of queuing behind this wait.
        let taken = self.slot.write().take();
        let Some(Open { core, memory }) = taken else {
            return Ok(());
        };
        drop(memory);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut core = core;
        loop {
            match Arc::try_unwrap(core) {
                Ok(core) => return core.close().map_err(napi_err),
                Err(shared) if std::time::Instant::now() < deadline => {
                    core = shared;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(shared) => {
                    let memory = Arc::new(self.reopen.memory_for(shared.clone())?);
                    self.slot.write().get_or_insert(Open {
                        core: shared,
                        memory,
                    });
                    return Err(napi::Error::from_reason(
                        "close timed out: operations are still running; the store stays open",
                    ));
                }
            }
        }
    }

    /// Replaces the memory API's configuration (policies and defaults; see
    /// docs/reference/api.md). Pass nothing to restore the defaults.
    #[napi]
    pub fn configure_memory(&self, config: Option<serde_json::Value>) -> Result<()> {
        let config: crate::core::memory_api::MemoryConfig = match config {
            Some(value) => serde_json::from_value(value)
                .map_err(|e| napi::Error::from_reason(format!("invalid memory config: {e}")))?,
            None => Default::default(),
        };
        let core = self.core()?;
        let memory =
            crate::core::memory_api::Memory::with_config(core, config.clone()).map_err(napi_err)?;
        match &mut *self.slot.write() {
            Some(open) => open.memory = Arc::new(memory),
            None => return Err(napi::Error::from_reason(CLOSED)),
        }
        *self.reopen.memory_config.lock() = Some(config);
        Ok(())
    }

    /// One memory API operation as JSON in and JSON out: `remember`, `recall`, `explain`,
    /// `forget`, `get`, `history`, `applyForgetting` or `consolidate`. The typed wrapper is
    /// `Memory` in `memory.js`.
    #[napi]
    pub async fn memory_call(
        &self,
        op: String,
        request: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let memory = self.memory()?;
        tokio::task::spawn_blocking(move || memory.call(&op, request))
            .await
            .map_err(|e| napi::Error::from_reason(e.to_string()))?
            .map_err(memory_err)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_document(&self, input: DocumentInput) -> Result<u32> {
        let core = self.core()?;
        run_async(move || core.memorize_document(input)).await
    }

    #[napi]
    pub async fn memorize_documents(&self, inputs: Vec<DocumentInput>) -> Result<Vec<u32>> {
        let core = self.core()?;
        run_async(move || core.memorize_documents(inputs)).await
    }

    /// Applies memorize and delete operations as one atomic unit: after a crash or in a
    /// read-only reader, all of the batch is visible or none of it. Operations apply in
    /// order; if any is invalid nothing is applied. 1..=4096 operations.
    #[napi]
    pub async fn write_batch(&self, ops: Vec<BatchOpJs>) -> Result<Vec<BatchResultJs>> {
        let core = self.core()?;
        run_async(move || {
            let ops = ops
                .into_iter()
                .map(BatchOpJs::into_op)
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(core
                .write_batch(ops)?
                .into_iter()
                .map(BatchResultJs::from)
                .collect())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn chunk_document(&self, input: DocumentInput) -> Result<Vec<DocumentChunk>> {
        run_async(move || chunk_document(&input)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn search_documents(
        &self,
        text: String,
        options: Option<SearchOptions>,
    ) -> Result<Vec<DocumentResult>> {
        let core = self.core()?;
        run_async(move || core.search_documents(&text, &options.unwrap_or_default())).await
    }

    /// Full-dimensional cosine results; optional graph prefixes only select candidates.
    /// Filters, stale projected snapshots and zero prefixes use full exact ranking.
    #[napi]
    pub async fn search_vectors(
        &self,
        embedding: Vec<f64>,
        k: f64,
        options: Option<VectorSearchOptions>,
    ) -> Result<Vec<DocumentResult>> {
        let core = self.core()?;
        run_async(move || {
            anyhow::ensure!(
                k.is_finite()
                    && k.fract() == 0.0
                    && (1.0..=crate::core::documents::MAX_K as f64).contains(&k),
                "k must be an integer in 1..=1000"
            );
            core.search_vectors(
                &embedding,
                k as usize,
                &options.unwrap_or_default().into_options()?,
            )
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn delete_document(&self, id: String) -> Result<bool> {
        let core = self.core()?;
        run_async(move || core.delete_document(&id)).await
    }

    /// One page of stored documents in id order, without their text. Pass the last id of the
    /// previous page as `after` to continue; `limit` is 1..=1000 (default 100).
    #[napi]
    pub async fn list_documents(
        &self,
        after: Option<String>,
        limit: Option<f64>,
    ) -> Result<Vec<ListedDocument>> {
        // An f64 so that a value outside u32 or a fraction is rejected, not wrapped or truncated.
        let limit = limit.unwrap_or(100.0);
        let core = self.core()?;
        run_async(move || {
            anyhow::ensure!(
                limit.fract() == 0.0 && (1.0..=1000.0).contains(&limit),
                "limit must be an integer in 1..=1000"
            );
            let limit = limit as usize;
            // Document ids cannot contain NUL, so `id + NUL` is the first key after `id`.
            let from = after.map(|id| format!("{id}\0"));
            Ok(core
                .list_document_info(from.as_deref(), limit)?
                .into_iter()
                .map(ListedDocument::from)
                .collect())
        })
        .await
    }

    #[napi]
    pub async fn get_document_info(&self, id: String) -> Result<Option<ListedDocument>> {
        let core = self.core()?;
        run_async(move || Ok(core.get_document_info(&id)?.map(ListedDocument::from))).await
    }

    /// Opaque store revision. Equal values bracket an unchanged store on this handle.
    #[napi]
    pub fn revision(&self) -> Result<String> {
        Ok(self.core()?.revision().to_string())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn security_status(&self) -> Result<serde_json::Value> {
        Ok(self.core()?.security_status())
    }

    #[napi(getter)]
    pub fn vector_count(&self) -> Result<u32> {
        Ok(self.core()?.vector_count() as u32)
    }

    #[napi(getter)]
    pub fn nsg_trained(&self) -> Result<bool> {
        Ok(self.core()?.nsg_trained())
    }

    #[napi(getter)]
    pub fn ivf_trained(&self) -> Result<bool> {
        Ok(self.core()?.ivf_trained())
    }

    #[napi(getter)]
    pub fn dimensions(&self) -> Result<u32> {
        Ok(self.core()?.dimensions() as u32)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn index_status(&self) -> Result<IndexStatus> {
        Ok(self.core()?.index_status())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn storage_health(&self) -> Result<StorageHealth> {
        Ok(self.core()?.storage_health())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn hardware_capabilities(&self) -> HardwareCapabilities {
        crate::core::hardware::capabilities()
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn explain_query(&self, text: String, k: u32) -> Result<QueryExplanation> {
        let query = self.core()?.try_encode_text(&text).map_err(napi_err)?;
        Ok(self.core()?.explain_query(&query, k))
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn maintain_indices(&self) -> Result<IndexStatus> {
        let core = self.core()?;
        run_async(move || core.maintain_indices()).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn flush(&self) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.flush()).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn analyze_text(&self, text: String) -> Result<TextMetrics> {
        let core = self.core()?;
        run_async(move || Ok(core.analyze_text(&text))).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn calculate_readability(&self, metrics: TextMetrics) -> Result<f64> {
        let core = self.core()?;
        run_async(move || Ok(core.calculate_readability(&metrics))).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_text(
        &self,
        id: String,
        text: String,
        trace_id: Option<String>,
    ) -> Result<()> {
        let core = self.core()?;
        run_async(move || {
            let _span =
                info_span!("memorize_text", id = %id, trace_id = trace_id.as_deref().unwrap_or(""))
                    .entered();
            let vec = core.try_encode_text(&text)?;
            core.memorize(id, vec)
        })
        .await
    }

    /// Zero-copy text ingestion from a Node.js Buffer. Avoids the UTF-8 copy
    /// that occurs with String parameters by reading bytes in-place.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_text_buffer(
        &self,
        id: String,
        text: Buffer,
        trace_id: Option<String>,
    ) -> Result<()> {
        let core = self.core()?;
        let text_str = std::str::from_utf8(&text)
            .map_err(|e| napi::Error::from_reason(format!("Invalid UTF-8: {}", e)))?
            .to_owned();
        run_async(move || {
            let _span = info_span!("memorize_text_buffer", id = %id, trace_id = trace_id.as_deref().unwrap_or("")).entered();
            let vec = core.try_encode_text(&text_str)?;
            core.memorize(id, vec)
        })
        .await
    }

    /// Batch memorize multiple id/text pairs in a single native call.
    /// Uses rayon for parallel encoding, then inserts sequentially.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_batch(
        &self,
        items: Vec<MemorizeBatchItem>,
        trace_id: Option<String>,
    ) -> Result<()> {
        let core = self.core()?;
        run_async(move || {
            let _span = info_span!(
                "memorize_batch",
                count = items.len(),
                trace_id = trace_id.as_deref().unwrap_or("")
            )
            .entered();
            core.memorize_batch(&items)
        })
        .await
    }

    /// Ingest a UTF-8 file (up to 8 MiB) as a chunked document with source offsets.
    /// Retrieve passages with searchDocuments and remove them with deleteDocument.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_file(&self, id: String, file_path: String) -> Result<()> {
        let core = self.core()?;
        run_async(move || {
            use std::io::Read;
            let file = std::fs::File::open(&file_path)
                .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", file_path, e))?;
            anyhow::ensure!(
                file.metadata()?.is_file(),
                "filePath must name a regular file"
            );
            let mut bytes = Vec::new();
            file.take(core::documents::MAX_DOCUMENT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() <= core::documents::MAX_DOCUMENT_BYTES,
                "file exceeds 8 MiB"
            );
            let text = String::from_utf8(bytes)?;
            core.memorize_document(DocumentInput {
                id,
                text,
                source_uri: Some(file_path),
                version: None,
                metadata: None,
                chunk_words: None,
                overlap_words: None,
                store_text: None,
                title_weight: None,
                embeddings: None,
            })?;
            Ok(())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_vector(&self, id: String, vector: Float32Array) -> Result<()> {
        let core = self.core()?;
        let dense: Vec<f32> = vector.to_vec();
        run_async(move || core.memorize_vector(id, &dense)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_scalar(&self, id: String, value: f64, min: f64, max: f64) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.memorize_scalar(id, value, min, max)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query(
        &self,
        text: String,
        k: u32,
        trace_id: Option<String>,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || {
            let _span =
                info_span!("query", k = k, trace_id = trace_id.as_deref().unwrap_or("")).entered();
            let q_vec = core.try_encode_text(&text)?;
            core.query(&q_vec, k)
        })
        .await
    }

    /// Converts float32→sparse EntangledHVec on JS thread, then queries on background.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_vector(&self, vector: Float32Array, k: u32) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        let dense = vector.to_vec();
        run_async(move || core.query_vector(&dense, k)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_scalar(
        &self,
        value: f64,
        min: f64,
        max: f64,
        k: u32,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || {
            let q_vec = EntangledHVec::from_scalar(value, min, max, core.dimensions());
            core.query(&q_vec, k)
        })
        .await
    }

    /// Process multiple text queries in parallel, returning results for each.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_batch(
        &self,
        texts: Vec<String>,
        k: u32,
    ) -> Result<Vec<Vec<RetrievalResult>>> {
        let core = self.core()?;
        run_async(move || {
            let queries: Vec<EntangledHVec> = texts
                .iter()
                .map(|t| core.try_encode_text(t))
                .collect::<anyhow::Result<_>>()?;
            core.query_batch(&queries, k)
        })
        .await
    }

    /// Process multiple float32 vector queries in parallel.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_vector_batch(
        &self,
        vectors: Vec<Float32Array>,
        k: u32,
    ) -> Result<Vec<Vec<RetrievalResult>>> {
        let core = self.core()?;
        let dense: Vec<Vec<f32>> = vectors.iter().map(|v| v.to_vec()).collect();
        run_async(move || {
            anyhow::ensure!(dense.len() <= 4096, "query batch exceeds 4096 vectors");
            use rayon::prelude::*;
            dense.par_iter().map(|v| core.query_vector(v, k)).collect()
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn analyze_components(&self, text: String) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || {
            let vec = core.try_encode_text(&text)?;
            core.analyze_components(&vec)
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn factorize_diffusion(
        &self,
        product_text: String,
        domains: Vec<Vec<String>>,
        max_iter: u32,
    ) -> Result<Vec<Option<String>>> {
        let core = self.core()?;
        run_async(move || {
            let vec = core.try_encode_text(&product_text)?;
            let domain_vecs: Vec<Vec<EntangledHVec>> = domains
                .iter()
                .map(|d| {
                    d.iter()
                        .map(|s| core.try_encode_text(s))
                        .collect::<anyhow::Result<_>>()
                })
                .collect::<anyhow::Result<_>>()?;
            let results = core.factorize_diffusion(&vec, &domain_vecs, max_iter as usize);

            let mapped = results
                .into_iter()
                .enumerate()
                .map(|(i, opt_vec)| {
                    opt_vec.and_then(|evec| {
                        domains[i]
                            .iter()
                            .zip(domain_vecs[i].iter())
                            .min_by_key(|(_, enc)| evec.hamming(enc))
                            .map(|(s, _)| s.clone())
                    })
                })
                .collect();
            Ok(mapped)
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_triplet(
        &self,
        id: String,
        head: String,
        relation: String,
        tail: String,
    ) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.memorize_triplet(id, head, relation, tail)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_triplet(
        &self,
        head: String,
        relation: String,
        k: u32,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || core.query_triplet(head, relation, k)).await
    }

    /// Finds an analogy: A is to B as C is to ?.
    ///
    /// NOTE: Currently uses character trigram encoding, which has limited semantic
    /// understanding. Complex semantic analogies may require higher-level word
    /// embeddings (slated for future upgrade).
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn find_analogy(
        &self,
        a: String,
        b: String,
        c: String,
        k: Option<u32>,
        trace_id: Option<String>,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || {
            let _span =
                info_span!("find_analogy", trace_id = trace_id.as_deref().unwrap_or("")).entered();
            let results = core.try_find_analogy(&a, &b, &c, k.unwrap_or(5))?;
            Ok(results)
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn synthesize_concepts(&self) -> Result<Vec<ConceptCandidate>> {
        let core = self.core()?;
        run_async(move || {
            core.synthesize_concepts_with_limits(crate::core::query_limits::QueryLimits::default())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_sequence(&self, id: String, sequence: Vec<String>) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.memorize_sequence(id, &sequence)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn train_nsg(&self) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.train_nsg()).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn train_ivf(&self) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.train_ivf()).await
    }

    #[napi]
    pub async fn train_document_index(&self) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.train_document_index()).await
    }

    #[napi(getter)]
    pub fn document_index_trained(&self) -> Result<bool> {
        Ok(self.core()?.document_index_trained())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn query_sequence(
        &self,
        partial: Vec<String>,
        k: u32,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || core.query_sequence(&partial, k)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn delete(&self, id: String) -> Result<bool> {
        let core = self.core()?;
        run_async(move || core.delete(&id)).await
    }

    /// Compacts the store. Searches keep running; writes wait for most of the call (its snapshot
    /// and its publication). The first compaction after a bulk ingest, and then whenever at least a
    /// quarter of the slots are new, also renumbers the document slots of a store without chunk
    /// vectors, which makes it 3 to 5 times as long (measured: 325 s against 64.5 s at 5M documents,
    /// 71.5 s against 17.7 s at 1M); with compressed text or encryption, searches also wait while
    /// the slots are renumbered in memory. `startCompaction` never renumbers.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn compact(&self) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.compact()).await
    }

    /// Starts a compaction on a worker thread and returns its job; reads and writes continue
    /// meanwhile. Same lifecycle as Rust `WarmCore::start_compaction`.
    #[napi(ts_args_type = "options?: CompactionOptions")]
    pub fn start_compaction(
        &self,
        options: Option<super::CompactionOptionsJs>,
    ) -> Result<super::CompactionJob> {
        let options = options.unwrap_or_default().into_options()?;
        let handle = self.core()?.start_compaction(options).map_err(napi_err)?;
        Ok(super::CompactionJob::new(handle))
    }

    /// Hexadecimal Ed25519 public key, or null when signing is disabled. Establish trust independently.
    #[napi]
    pub fn audit_verifying_key(&self) -> Result<Option<String>> {
        Ok({
            self.core()?
                .audit_verifying_key()
                .map(|key| super::types::hex_encode(&key))
        })
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn audit_since(&self, timestamp_ms: f64) -> Result<Vec<AuditEntryJs>> {
        if !timestamp_ms.is_finite()
            || timestamp_ms.fract() != 0.0
            || !(0.0..=9_007_199_254_740_991.0).contains(&timestamp_ms)
        {
            return Err(napi::Error::from_reason(
                "timestampMs must be a nonnegative safe integer",
            ));
        }
        let core = self.core()?;
        let ts = timestamp_ms as u64;
        run_async(move || {
            let entries = core.audit_since(ts)?;
            Ok(entries.into_iter().map(AuditEntryJs::from).collect())
        })
        .await
    }

    /// Bundle multiple text items into a single hypervector.
    /// Respects the PrivacyConfig: when dp_enabled, uses epsilon-DP noise.
    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn bundle_texts(&self, texts: Vec<String>) -> Result<Vec<u32>> {
        let core = self.core()?;
        run_async(move || {
            let vecs: Vec<EntangledHVec> = texts
                .iter()
                .map(|t| core.try_encode_text(t))
                .collect::<anyhow::Result<_>>()?;
            let bundled = core.bundle(&vecs);
            Ok(bundled.indices().to_vec())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn memorize_meaning(&self, id: String, text: String) -> Result<()> {
        let core = self.core()?;
        run_async(move || core.memorize_meaning(&id, &text)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn structural_query(
        &self,
        known_subjects: Vec<String>,
        known_relations: Vec<String>,
        target_role: String,
    ) -> Result<Vec<StructuralResultJs>> {
        let core = self.core()?;
        run_async(move || {
            let known_vecs: Vec<EntangledHVec> = known_subjects
                .iter()
                .chain(known_relations.iter())
                .map(|t| core.try_encode_text(t))
                .collect::<anyhow::Result<_>>()?;
            let mut bindings: Vec<(&str, &EntangledHVec)> = Vec::new();
            for (i, v) in known_vecs.iter().enumerate() {
                if i < known_subjects.len() {
                    bindings.push(("subject", v));
                } else {
                    bindings.push(("relation", v));
                }
            }
            let results = core.structural_query(&bindings, &target_role);
            Ok(results
                .into_iter()
                .map(|r| StructuralResultJs {
                    entity_id: r.entity_id,
                    confidence: r.confidence,
                    path: format!("{:?}", r.path),
                })
                .collect())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn multi_hop_query(
        &self,
        start_entity: String,
        relations: Vec<String>,
    ) -> Result<Vec<MultiHopResultJs>> {
        let core = self.core()?;
        run_async(move || {
            let rel_refs: Vec<&str> = relations.iter().map(|s| s.as_str()).collect();
            let results = core.multi_hop_with_limits(
                &start_entity,
                &rel_refs,
                crate::core::query_limits::QueryLimits::default(),
            )?;
            Ok(results
                .into_iter()
                .map(|r| MultiHopResultJs {
                    entity_id: r.entity_id,
                    confidence: r.confidence,
                    method: format!("{:?}", r.method),
                })
                .collect())
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn meaning_cleanup(&self, text: String) -> Result<Option<CleanupResultJs>> {
        let core = self.core()?;
        run_async(move || {
            let vec = core.try_encode_text(&text)?;
            Ok(core
                .meaning_cleanup(&vec)
                .map(|(id, confidence)| CleanupResultJs { id, confidence }))
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn cleanup_vector(&self, vector: Float32Array) -> Result<Option<CleanupResultJs>> {
        let core = self.core()?;
        let q_vec = EntangledHVec::from_dense(&vector, core.dimensions());
        run_async(move || {
            Ok(core
                .meaning_cleanup(&q_vec)
                .map(|(id, confidence)| CleanupResultJs { id, confidence }))
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn declare_composition_rule(
        &self,
        name: String,
        input_relations: Vec<String>,
        output_relation: String,
    ) -> Result<()> {
        self.core()?
            .declare_rule(&name, input_relations, output_relation)
            .map_err(napi_err)
    }

    #[napi(getter)]
    pub fn meaning_enabled(&self) -> Result<bool> {
        Ok(self.core()?.meaning_enabled())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn start_cognition(&self) -> Result<()> {
        self.core()?.start_cognition().map_err(napi_err)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn stop_cognition(&self) -> Result<()> {
        self.core()?.stop_cognition();
        Ok(())
    }

    #[napi(getter)]
    pub fn cognition_running(&self) -> Result<bool> {
        Ok(self.core()?.cognition_running())
    }

    #[napi(getter)]
    pub fn cognition_cycle_count(&self) -> Result<u32> {
        Ok(self.core()?.cognition_cycle_count() as u32)
    }

    #[napi(getter)]
    pub fn cognition_insight_count(&self) -> Result<u32> {
        Ok(self.core()?.cognition_insight_count() as u32)
    }

    #[napi(getter)]
    pub fn cognition_enabled(&self) -> Result<bool> {
        Ok(self.core()?.cognition_enabled())
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn run_cognition_once(&self) -> Result<u32> {
        Ok(self.core()?.run_cognition_once().len() as u32)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn govern_memory(&self) -> Result<GovernanceReportJs> {
        self.core()?
            .govern_memory()
            .map(governance_report)
            .map_err(napi_err)
    }

    #[napi]
    pub async fn govern_memory_async(&self) -> Result<GovernanceReportJs> {
        let core = self.core()?;
        run_async(move || core.govern_memory().map(governance_report)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn add_goal(
        &self,
        name: String,
        description: String,
        relevance: f64,
        urgency: f64,
        cost: f64,
    ) -> Result<Option<u32>> {
        Ok(self
            .core()?
            .add_goal(&name, &description, relevance, urgency, cost)
            .map(|id| id as u32))
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn deactivate_goal(&self, name: String) -> Result<bool> {
        Ok(self.core()?.deactivate_goal(&name))
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn active_goals(&self) -> Result<Vec<GoalJs>> {
        Ok({
            self.core()?
                .active_goals()
                .into_iter()
                .map(|(name, utility)| GoalJs { name, utility })
                .collect()
        })
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn plan_goal(
        &self,
        goal: String,
        causal_relations: Vec<String>,
        max_depth: Option<u32>,
    ) -> Result<PlanJs> {
        let rel_refs: Vec<&str> = causal_relations.iter().map(|s| s.as_str()).collect();
        let plan = self
            .core()?
            .plan_goal(&goal, &rel_refs, max_depth.unwrap_or(10) as usize);
        Ok(PlanJs {
            goal: plan.goal,
            actions: plan
                .actions
                .into_iter()
                .map(|a| PlannedActionJs {
                    subject: a.subject,
                    relation: a.relation,
                    object: a.object,
                    depth: a.depth as u32,
                    cost: a.cost,
                })
                .collect(),
            complete: plan.complete,
            total_cost: plan.total_cost,
        })
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn generate_questions(&self) -> Result<Vec<QuestionJs>> {
        Ok({
            self.core()?
                .generate_questions()
                .into_iter()
                .map(|q| QuestionJs {
                    text: q.text,
                    goal_relevance: q.goal_relevance,
                })
                .collect()
        })
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn propose_rule(
        &self,
        name: String,
        input_relations: Vec<String>,
        output_relation: String,
        reason: String,
    ) -> Result<Option<u32>> {
        Ok(self
            .core()?
            .propose_rule(&name, input_relations, &output_relation, &reason)
            .map(|id| id as u32))
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn approve_proposal(&self, id: u32) -> Result<bool> {
        Ok(self.core()?.approve_proposal(id as usize))
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn reject_proposal(&self, id: u32) -> Result<bool> {
        Ok(self.core()?.reject_proposal(id as usize))
    }

    #[napi(getter)]
    pub fn pending_proposals(&self) -> Result<u32> {
        Ok(self.core()?.pending_proposals() as u32)
    }

    #[napi(getter)]
    pub fn goal_count(&self) -> Result<u32> {
        Ok(self.core()?.goal_count() as u32)
    }

    #[napi(getter)]
    pub fn active_goal_count(&self) -> Result<u32> {
        Ok(self.core()?.active_goal_count() as u32)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn add_relation(
        &self,
        source_id: String,
        relation_type: String,
        target_id: String,
        properties: Option<String>,
        valid_from: Option<f64>,
        valid_to: Option<f64>,
    ) -> Result<()> {
        let core = self.core()?;
        run_async(move || {
            let rel = crate::core::types::Relation {
                source_id,
                relation_type,
                target_id,
                properties,
                valid_from: valid_from.unwrap_or(0.0),
                valid_to: valid_to.unwrap_or(0.0),
            };
            core.add_relation(&rel)
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn remove_relation(
        &self,
        source_id: String,
        relation_type: String,
        target_id: String,
    ) -> Result<bool> {
        let core = self.core()?;
        run_async(move || core.remove_relation(&source_id, &relation_type, &target_id)).await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub fn declare_relation_type(
        &self,
        name: String,
        transitive: Option<bool>,
        symmetric: Option<bool>,
    ) -> Result<()> {
        self.core()?
            .declare_relation_type(crate::core::types::RelationType {
                name,
                transitive: transitive.unwrap_or(false),
                symmetric: symmetric.unwrap_or(false),
            })
            .map_err(napi_err)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn traverse(
        &self,
        start_id: String,
        relation_type: Option<String>,
        max_depth: Option<u32>,
        at_time: Option<f64>,
    ) -> Result<Vec<crate::core::types::GraphPath>> {
        let core = self.core()?;
        run_async(move || {
            Ok(core.traverse(
                &start_id,
                relation_type.as_deref(),
                max_depth.unwrap_or(3),
                at_time.unwrap_or(0.0),
            ))
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn outgoing_relations(
        &self,
        source_id: String,
        relation_type: Option<String>,
        at_time: Option<f64>,
    ) -> Result<Vec<crate::core::types::Relation>> {
        let core = self.core()?;
        run_async(move || {
            Ok(core.outgoing_relations(
                &source_id,
                relation_type.as_deref(),
                at_time.unwrap_or(0.0),
            ))
        })
        .await
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn incoming_relations(
        &self,
        target_id: String,
        relation_type: Option<String>,
        at_time: Option<f64>,
    ) -> Result<Vec<crate::core::types::Relation>> {
        let core = self.core()?;
        run_async(move || {
            Ok(core.incoming_relations(
                &target_id,
                relation_type.as_deref(),
                at_time.unwrap_or(0.0),
            ))
        })
        .await
    }

    #[napi(getter)]
    pub fn relation_count(&self) -> Result<u32> {
        Ok(self.core()?.relation_count() as u32)
    }

    #[napi]
    // codeql[rust/access-invalid-pointer] False positive: NAPI macro generated FFI bindings
    pub async fn federated_query(
        &self,
        peer_paths: Vec<String>,
        text: String,
        k: u32,
    ) -> Result<Vec<RetrievalResult>> {
        let core = self.core()?;
        run_async(move || {
            let q_vec = core.try_encode_text(&text)?;
            core.federated_query(&peer_paths, &q_vec, k)
        })
        .await
    }
}
