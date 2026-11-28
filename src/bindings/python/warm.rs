// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Python surface of the in-process engine: `Warm`, a store with document search
//! (`memorize_document`, `search_documents`), the memory API (`remember`, `recall`, `forget`,
//! `explain`, `get`, `history`) and vector search (`memorize_vector`, `query_vector`). Requests
//! and results are plain Python dicts and lists with the camelCase fields of the Node
//! binding. Calls release the GIL.
//!
//! Recall is hybrid (BM25 blended with exact cosine) when the store has vectors: open it with
//! an `embedder` (build `local-embed`) so the engine embeds facts and queries itself, or with
//! `embedding_dimensions` and pass `embedding` on `remember` and `recall`. A store opened with
//! neither ranks lexically, exactly as before.

use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use serde_json::Value;

use pyo3::create_exception;

create_exception!(
    writerslogic_warm,
    StoreClosedError,
    PyValueError,
    "The store was used after `close()`."
);
create_exception!(
    writerslogic_warm,
    ReadOnlyError,
    PyValueError,
    "A mutation was attempted on a store opened with `Warm.open_read_only`."
);
create_exception!(
    writerslogic_warm,
    StoreLockedError,
    PyValueError,
    "Another process holds the store for writing; the message names it."
);

/// The error class of `e`, matched on the typed engine errors' messages.
fn err(e: impl std::fmt::Display) -> PyErr {
    let text = format!("{e:#}");
    if text.contains(&crate::ReadOnlyError.to_string()) {
        ReadOnlyError::new_err(text)
    } else if text.contains("is already open by another engine") {
        StoreLockedError::new_err(text)
    } else {
        super::context_err(text)
    }
}
use crate::core::config::WarmConfig;
use crate::core::documents::{DocumentInput, IngestOptions, SearchOptions};
use crate::core::engine::{RankingMode, SparseSearch};
use crate::core::index::sparse_store::SparseVector;
use crate::core::memory_api::Memory;
use crate::core::schema::EmbeddingSpace;
use crate::WarmCore;

/// JSON text of `object`. Numpy arrays and scalars serialize through `tolist`, and NaN or
/// infinity is an error here rather than a malformed document later.
fn to_value(py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<Value> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("allow_nan", false)?;
    kwargs.set_item(
        "default",
        py.import("operator")?
            .call_method1("methodcaller", ("tolist",))?,
    )?;
    let text: String = py
        .import("json")?
        .call_method("dumps", (object,), Some(&kwargs))
        .map_err(|e| {
            if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
                PyValueError::new_err("request holds a value that is not JSON serializable")
            } else {
                e
            }
        })?
        .extract()?;
    serde_json::from_str(&text).map_err(err)
}

fn to_py<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    let text = serde_json::to_string(value).map_err(err)?;
    py.import("json")?.call_method1("loads", (text,))
}

fn typed<T: serde::de::DeserializeOwned>(py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<T> {
    serde_json::from_value(to_value(py, object)?).map_err(err)
}

#[cfg(feature = "local-embed")]
fn installed_embedder(object: &Bound<'_, PyAny>) -> PyResult<Arc<crate::core::embed::Embedder>> {
    let embedder: PyRef<'_, super::embed::PyEmbedder> = object
        .extract()
        .map_err(|_| PyValueError::new_err("embedder must be a writerslogic_warm.Embedder"))?;
    Ok(embedder.inner.clone())
}

/// An in-process WARM store: document search, the memory API and vector search.
#[pyclass(name = "Warm")]
struct PyWarm {
    /// `None` once closed. Calls clone the `Arc`s out and run on the clones, so `close` waits
    /// for them before it releases the store lock.
    slot: std::sync::RwLock<Option<Open>>,
    reopen: Reopen,
}

struct Open {
    core: Arc<WarmCore>,
    memory: Arc<Memory>,
}

/// What `refresh` needs to open the store again.
struct Reopen {
    read_only: bool,
    dimensions: u32,
    path: Option<String>,
    config: WarmConfig,
    #[cfg(feature = "local-embed")]
    embedder: Option<Arc<crate::core::embed::Embedder>>,
}

impl PyWarm {
    fn slot(&self) -> std::sync::RwLockReadGuard<'_, Option<Open>> {
        self.slot.read().unwrap_or_else(|e| e.into_inner())
    }

    fn core(&self) -> PyResult<Arc<WarmCore>> {
        self.slot()
            .as_ref()
            .map(|open| open.core.clone())
            .ok_or_else(|| StoreClosedError::new_err("the store is closed"))
    }

    fn memory(&self) -> PyResult<Arc<Memory>> {
        self.slot()
            .as_ref()
            .map(|open| open.memory.clone())
            .ok_or_else(|| StoreClosedError::new_err("the store is closed"))
    }

    fn open_engine(reopen: &Reopen) -> anyhow::Result<Open> {
        let core = if reopen.read_only {
            WarmCore::open_read_only(
                reopen.dimensions,
                reopen.path.clone(),
                Some(reopen.config.clone()),
            )?
        } else {
            WarmCore::new(
                reopen.dimensions,
                reopen.path.clone(),
                Some(reopen.config.clone()),
            )?
        };
        #[cfg(feature = "local-embed")]
        core.set_text_embedder(reopen.embedder.clone())?;
        let core = Arc::new(core);
        Ok(Open {
            memory: Arc::new(Memory::new(core.clone())),
            core,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        py: Python<'_>,
        read_only: bool,
        dimensions: u32,
        path: Option<String>,
        embedder: Option<&Bound<'_, PyAny>>,
        embedding_dimensions: Option<usize>,
        embedding_model: &str,
        embedding_revision: &str,
        experience_form_state: bool,
        master_key_path: Option<String>,
    ) -> PyResult<Self> {
        if embedder.is_some() && embedding_dimensions.is_some() {
            return Err(err(
                "pass either embedder or embedding_dimensions, not both: an embedder fixes the embedding space",
            ));
        }
        let mut config = WarmConfig::default();
        config.experience.form_state = experience_form_state;
        config.security.master_key_file = master_key_path;
        #[cfg(feature = "local-embed")]
        let mut text_embedder = None;
        if let Some(embedder) = embedder {
            #[cfg(feature = "local-embed")]
            {
                let embedder = installed_embedder(embedder)?;
                config.embedding_space = Some(embedder.space());
                text_embedder = Some(embedder);
            }
            #[cfg(not(feature = "local-embed"))]
            return Err(err(format!(
                "this build has no local embedder (feature local-embed); got {embedder}"
            )));
        } else if let Some(dimensions) = embedding_dimensions {
            config.embedding_space = Some(EmbeddingSpace {
                model: embedding_model.into(),
                revision: embedding_revision.into(),
                dimensions,
                normalization: "l2".into(),
                metric: "cosine".into(),
            });
        }
        let reopen = Reopen {
            read_only,
            dimensions,
            path,
            config,
            #[cfg(feature = "local-embed")]
            embedder: text_embedder,
        };
        let open = py.detach(|| Self::open_engine(&reopen)).map_err(err)?;
        Ok(Self {
            slot: std::sync::RwLock::new(Some(open)),
            reopen,
        })
    }
}

#[pymethods]
impl PyWarm {
    /// Opens or creates the store at `path` (the current directory when `None`).
    /// `dimensions` is the sparse hypervector dimension of the store.
    ///
    /// Dense vectors, which make recall and document search hybrid, come from one of two
    /// places. `embedder` (a `writerslogic_warm.Embedder`, build `local-embed`) is installed in
    /// the engine, which then embeds every fact and query itself; the store's embedding space
    /// is the embedder's. `embedding_dimensions` instead declares an external embedding space,
    /// named by `embedding_model` and `embedding_revision`, and callers pass `embedding` on
    /// `remember` and `recall`. The two are exclusive. Reopening a store needs the same choice.
    ///
    /// `experience_form_state` parses the form controls of an observation from the
    /// `accessibilityTree` text it carries and stores them with the step (see
    /// `experience_form_tables`); off, that text is not parsed but is still refused over
    /// 4 MiB.
    ///
    /// `master_key_path` is the file holding the master key of an encrypted store; it falls
    /// back to `WARM_MASTER_KEY_PATH`, then `<path>/master.key`. A new store creates it (mode
    /// 0600); an existing store refuses a missing or different key.
    #[new]
    #[pyo3(signature = (
        dimensions=16384, path=None, *, embedder=None, embedding_dimensions=None,
        embedding_model="external", embedding_revision="1", experience_form_state=true,
        master_key_path=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        dimensions: u32,
        path: Option<String>,
        embedder: Option<&Bound<'_, PyAny>>,
        embedding_dimensions: Option<usize>,
        embedding_model: &str,
        embedding_revision: &str,
        experience_form_state: bool,
        master_key_path: Option<String>,
    ) -> PyResult<Self> {
        Self::build(
            py,
            false,
            dimensions,
            path,
            embedder,
            embedding_dimensions,
            embedding_model,
            embedding_revision,
            experience_form_state,
            master_key_path,
        )
    }

    /// Opens an existing store without the writer lock, so another process can hold it for
    /// writing. Reads see one committed snapshot; `refresh()` moves to a newer one. Every
    /// mutation raises `ReadOnlyError`. Takes the same arguments as the constructor.
    #[staticmethod]
    #[pyo3(signature = (
        dimensions=16384, path=None, *, embedder=None, embedding_dimensions=None,
        embedding_model="external", embedding_revision="1", experience_form_state=true,
        master_key_path=None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn open_read_only(
        py: Python<'_>,
        dimensions: u32,
        path: Option<String>,
        embedder: Option<&Bound<'_, PyAny>>,
        embedding_dimensions: Option<usize>,
        embedding_model: &str,
        embedding_revision: &str,
        experience_form_state: bool,
        master_key_path: Option<String>,
    ) -> PyResult<Self> {
        Self::build(
            py,
            true,
            dimensions,
            path,
            embedder,
            embedding_dimensions,
            embedding_model,
            embedding_revision,
            experience_form_state,
            master_key_path,
        )
    }

    /// True for a store opened with `open_read_only`.
    #[getter]
    fn read_only(&self) -> bool {
        self.reopen.read_only
    }

    /// True after `close()`.
    #[getter]
    fn closed(&self) -> bool {
        self.slot().is_none()
    }

    /// Moves a read-only store to the writer's latest commit, reading only what was appended
    /// since (a compaction is read once, afresh). Returns False when nothing was committed.
    fn refresh(&self, py: Python<'_>) -> PyResult<bool> {
        if !self.reopen.read_only {
            return Err(err("refresh applies to a store opened read-only"));
        }
        let current = self.core()?;
        let reopen = &self.reopen;
        let fresh = py
            .detach(|| {
                match current.refresh_in_place()? {
                    crate::core::Refresh::Unchanged => anyhow::Ok(Some(None)),
                    crate::core::Refresh::Advanced => Ok(None),
                    // A compaction replaced the generation this store reads: open it afresh.
                    crate::core::Refresh::Reopen => Ok(Some(Some(Self::open_engine(reopen)?))),
                }
            })
            .map_err(err)?;
        let fresh = match fresh {
            None => return Ok(true),
            Some(None) => return Ok(false),
            Some(Some(fresh)) => fresh,
        };
        match &mut *self.slot.write().unwrap_or_else(|e| e.into_inner()) {
            Some(open) => {
                *open = fresh;
                Ok(true)
            }
            None => Err(StoreClosedError::new_err("the store is closed")),
        }
    }

    /// Flushes and releases the store lock so another process can open it for writing. Safe
    /// to call twice. Every later call raises `StoreClosedError`. Waits up to 10 seconds for
    /// calls in flight; if they do not finish the store stays open and this raises.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        // Take the engine out and release the lock on the slot before waiting: calls that
        // arrive meanwhile fail with `StoreClosedError` at once instead of queuing behind
        // this wait while holding the GIL that the calls in flight need to finish.
        let taken = self.slot.write().unwrap_or_else(|e| e.into_inner()).take();
        let Some(Open { core, memory }) = taken else {
            return Ok(());
        };
        drop(memory);
        let outcome = py.detach(|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut core = core;
            loop {
                match Arc::try_unwrap(core) {
                    Ok(core) => return Ok(core.close()),
                    Err(shared) if std::time::Instant::now() < deadline => {
                        core = shared;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(shared) => return Err(shared),
                }
            }
        });
        match outcome {
            Ok(closed) => closed.map_err(err),
            Err(shared) => {
                let memory = Arc::new(Memory::new(shared.clone()));
                self.slot
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_or_insert(Open {
                        core: shared,
                        memory,
                    });
                Err(err(
                    "close timed out: operations are still running; the store stays open",
                ))
            }
        }
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        slf.core()?;
        Ok(slf)
    }

    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close(py)?;
        Ok(false)
    }

    /// The embedding model the store was built with as a dict (`model`, `revision`,
    /// `dimensions`, `normalization`, `metric`, `packSha256`), or `None` without an embedding
    /// space. `packSha256` is `None` while no embedder is installed; compare `revision` and
    /// `dimensions` with your embedder's to detect a mismatch.
    fn embedding_model<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.core()?
            .embedding_model()
            .map(|info| to_py(py, &serde_json::to_value(info).map_err(err)?))
            .transpose()
    }

    /// Adds or replaces one document: `{"id", "text", "metadata"?, "embeddings"?, ...}`.
    /// Returns the number of chunks stored.
    fn memorize_document(&self, py: Python<'_>, document: &Bound<'_, PyAny>) -> PyResult<u32> {
        let input: DocumentInput = typed(py, document)?;
        let core = self.core()?;
        py.detach(|| core.memorize_document(input)).map_err(err)
    }

    /// [`Self::memorize_document`] for a list of documents in one transaction.
    fn memorize_documents(
        &self,
        py: Python<'_>,
        documents: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<u32>> {
        let inputs: Vec<DocumentInput> = typed(py, documents)?;
        let core = self.core()?;
        py.detach(|| core.memorize_documents(inputs)).map_err(err)
    }

    /// Applies `[{"op": "memorize", "document": {...}}, {"op": "delete", "id": ...}]` as one
    /// atomic unit: after a crash or in a read-only reader, all of it is visible or none.
    /// Operations apply in order; if any is invalid nothing is applied. 1..=4096 operations.
    /// Returns `[{"op": "memorize", "chunks": n}, {"op": "delete", "existed": bool}, ...]`.
    fn write_batch<'py>(
        &self,
        py: Python<'py>,
        ops: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ops: Vec<crate::core::engine::BatchOp> = typed(py, ops)?;
        let core = self.core()?;
        let results = py.detach(|| core.write_batch(ops)).map_err(err)?;
        to_py(py, &serde_json::to_value(results).map_err(err)?)
    }

    /// Hybrid lexical (BM25) and semantic search over the documents. `options` takes `k`,
    /// `filter`, `embedding`, `lexicalWeight`, `semanticWeight` and the other camelCase fields
    /// of the document search options. Returns a list of result dicts, best first.
    #[pyo3(signature = (query, options=None))]
    fn search_documents<'py>(
        &self,
        py: Python<'py>,
        query: &str,
        options: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options: SearchOptions = match options {
            Some(options) => typed(py, options)?,
            None => SearchOptions::default(),
        };
        let core = self.core()?;
        let hits = py
            .detach(|| core.search_documents(query, &options))
            .map_err(err)?;
        to_py(py, &serde_json::to_value(hits).map_err(err)?)
    }

    /// [`Self::memorize_documents`] with learned-sparse vectors: `sparse` is a list parallel to
    /// `documents` whose entries are `None` or an `(ids, weights)` pair of term ids and
    /// non-negative weights. A document with `None` has no sparse vector afterwards. Not
    /// supported on an encrypted store.
    fn memorize_documents_sparse(
        &self,
        py: Python<'_>,
        documents: &Bound<'_, PyAny>,
        sparse: Vec<Option<(Vec<u32>, Vec<f32>)>>,
    ) -> PyResult<Vec<u32>> {
        let documents: Vec<DocumentInput> = typed(py, documents)?;
        if documents.len() != sparse.len() {
            return Err(err(format!(
                "{} documents but {} sparse entries",
                documents.len(),
                sparse.len()
            )));
        }
        let inputs = documents
            .into_iter()
            .zip(sparse)
            .map(|(document, vector)| {
                (
                    document,
                    vector.map(|(ids, weights)| SparseVector::new(ids, weights)),
                )
            })
            .collect();
        let core = self.core()?;
        py.detach(|| core.memorize_documents_sparse(inputs, &IngestOptions::default()))
            .map_err(err)
    }

    /// [`Self::search_documents`] under a ranking: `"lexical"` (default, ignores
    /// `sparse_query`), `"sparse"` (inner product with the stored sparse vectors) or `"fused"`
    /// (reciprocal rank fusion of both). `sparse_query` is an `(ids, weights)` pair.
    #[pyo3(signature = (query, options=None, ranking="lexical", sparse_query=None))]
    fn search_documents_ranked<'py>(
        &self,
        py: Python<'py>,
        query: &str,
        options: Option<&Bound<'py, PyAny>>,
        ranking: &str,
        sparse_query: Option<(Vec<u32>, Vec<f32>)>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options: SearchOptions = match options {
            Some(options) => typed(py, options)?,
            None => SearchOptions::default(),
        };
        let mode: RankingMode = ranking.parse().map_err(err)?;
        let (ids, weights) = sparse_query.unwrap_or_default();
        if mode != RankingMode::Lexical && ids.is_empty() {
            return Err(err("sparse_query is required for sparse and fused ranking"));
        }
        let ranking = SparseSearch {
            mode,
            query: SparseVector::new(ids, weights),
        };
        let core = self.core()?;
        let hits = py
            .detach(|| core.search_documents_ranked(query, &options, &ranking))
            .map_err(err)?;
        to_py(py, &serde_json::to_value(hits).map_err(err)?)
    }

    fn delete_document(&self, py: Python<'_>, id: &str) -> PyResult<bool> {
        let core = self.core()?;
        py.detach(|| core.delete_document(id)).map_err(err)
    }

    fn get_document<'py>(&self, py: Python<'py>, id: &str) -> PyResult<Bound<'py, PyAny>> {
        let core = self.core()?;
        let record = py.detach(|| core.get_document(id)).map_err(err)?;
        to_py(py, &serde_json::to_value(record).map_err(err)?)
    }

    /// Stores a fact: `{"text", "id"?, "eventTime"?, "validFrom"?, "validTo"?, "subject"?, ...}`.
    fn remember<'py>(
        &self,
        py: Python<'py>,
        fact: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, "remember", fact)
    }

    /// The facts relevant to `query`, best first. `options` takes `k`, `asOf`, `knownAt`,
    /// `subject`, `entities` and the other recall fields.
    #[pyo3(signature = (query, options=None))]
    fn recall<'py>(
        &self,
        py: Python<'py>,
        query: &str,
        options: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut request = match options {
            Some(options) => to_value(py, options)?,
            None => Value::Object(Default::default()),
        };
        let object = request
            .as_object_mut()
            .ok_or_else(|| PyValueError::new_err("options must be a dict"))?;
        object.insert("query".into(), query.into());
        self.dispatch(py, "recall", request)
    }

    fn forget<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, "forget", request)
    }

    fn explain<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, "explain", request)
    }

    /// Any other memory API operation by name (`get`, `history`, `applyForgetting`,
    /// `consolidate`), with a dict request.
    fn memory_call<'py>(
        &self,
        py: Python<'py>,
        op: &str,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.call(py, op, request)
    }

    /// The form tables of observation steps, one string per `(episode, step)` in order: the
    /// step's controls as rows, without rows already shown for an earlier step of the list, or
    /// `""` for a step with no controls.
    fn experience_form_tables(
        &self,
        py: Python<'_>,
        states: Vec<(String, u32)>,
    ) -> PyResult<Vec<String>> {
        let core = self.core()?;
        Ok(py.detach(|| core.experience_form_tables(&states)))
    }

    /// The action stored with an observation step and the occurrences it targeted, or `None`.
    /// `record` is the action as given; each target has its `role`, its `occurrence` (episode
    /// uid, step and child path in the previous observation) and the `node` there, or no
    /// occurrence when the record says it was unresolved.
    fn experience_action_targets<'py>(
        &self,
        py: Python<'py>,
        episode: String,
        step: u32,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let core = self.core()?;
        let v = py.detach(|| core.experience_action_targets(&episode, step));
        v.map(|v| to_py(py, &v)).transpose()
    }

    /// Stores `vector` under `id` in the vector index (the vertex graph).
    fn memorize_vector(&self, py: Python<'_>, id: String, vector: Vec<f32>) -> PyResult<()> {
        let core = self.core()?;
        py.detach(|| core.memorize_vector(id, &vector)).map_err(err)
    }

    /// The `k` nearest stored vectors as `(id, cosine)` pairs, best first.
    #[pyo3(signature = (vector, k=10))]
    fn query_vector(
        &self,
        py: Python<'_>,
        vector: Vec<f32>,
        k: u32,
    ) -> PyResult<Vec<(String, f64)>> {
        let core = self.core()?;
        let hits = py.detach(|| core.query_vector(&vector, k)).map_err(err)?;
        Ok(hits.into_iter().map(|h| (h.id, h.similarity)).collect())
    }
}

impl PyWarm {
    fn call<'py>(
        &self,
        py: Python<'py>,
        op: &str,
        request: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = to_value(py, request)?;
        self.dispatch(py, op, request)
    }

    fn dispatch<'py>(
        &self,
        py: Python<'py>,
        op: &str,
        request: Value,
    ) -> PyResult<Bound<'py, PyAny>> {
        let memory = self.memory()?;
        let result = py.detach(|| memory.call(op, request)).map_err(err)?;
        to_py(py, &result)
    }
}

fn analyzer_named(name: Option<&str>) -> PyResult<crate::core::documents::Analyzer> {
    name.map_or_else(|| Ok(Default::default()), |name| name.parse().map_err(err))
}

/// The indexed term of one word, stemmed as the indexer does. `analyzer` names the store's
/// analyzer (default: the legacy English analyzer).
#[pyfunction]
#[pyo3(signature = (word, analyzer=None))]
fn stem(word: &str, analyzer: Option<&str>) -> PyResult<String> {
    Ok(analyzer_named(analyzer)?.stem(word))
}

/// The indexed terms of `text`, in order with repeats, as the indexer counts them.
#[pyfunction]
#[pyo3(signature = (text, analyzer=None))]
fn analyze(text: &str, analyzer: Option<&str>) -> PyResult<Vec<String>> {
    Ok(analyzer_named(analyzer)?.analyze(text))
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(stem, m)?)?;
    m.add_function(wrap_pyfunction!(analyze, m)?)?;
    m.add_class::<PyWarm>()?;
    m.add("StoreClosedError", m.py().get_type::<StoreClosedError>())?;
    m.add("ReadOnlyError", m.py().get_type::<ReadOnlyError>())?;
    m.add("StoreLockedError", m.py().get_type::<StoreLockedError>())?;
    Ok(())
}
