// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Python surface of the in-process engine: `Warm`, a store with document search
//! (`memorize_document`, `search_documents`), the memory API (`remember`, `recall`, `forget`,
//! `explain`, `get`, `history`) and vector search (`memorize_vector`, `query_vector`). Requests
//! and results are plain Python dicts and lists with the camelCase fields of the Node
//! binding. Calls release the GIL.

use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyAny;
use serde_json::Value;

use crate::core::documents::{DocumentInput, IngestOptions, SearchOptions};
use crate::core::engine::{RankingMode, SparseSearch};
use crate::core::index::sparse_store::SparseVector;
use crate::core::memory_api::Memory;
use crate::WarmCore;

fn err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("{e:#}"))
}

fn to_value(py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<Value> {
    let text: String = py
        .import("json")?
        .call_method1("dumps", (object,))?
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

/// An in-process WARM store: document search, the memory API and vector search.
#[pyclass(name = "Warm")]
struct PyWarm {
    core: Arc<WarmCore>,
    memory: Arc<Memory>,
}

#[pymethods]
impl PyWarm {
    /// Opens or creates the store at `path` (the current directory when `None`).
    /// `dimensions` is the sparse hypervector dimension of the store.
    #[new]
    #[pyo3(signature = (dimensions=16384, path=None))]
    fn new(py: Python<'_>, dimensions: u32, path: Option<String>) -> PyResult<Self> {
        let core = py
            .detach(|| WarmCore::new(dimensions, path, None))
            .map_err(err)?;
        let core = Arc::new(core);
        Ok(Self {
            memory: Arc::new(Memory::new(core.clone())),
            core,
        })
    }

    /// Adds or replaces one document: `{"id", "text", "metadata"?, "embeddings"?, ...}`.
    /// Returns the number of chunks stored.
    fn memorize_document(&self, py: Python<'_>, document: &Bound<'_, PyAny>) -> PyResult<u32> {
        let input: DocumentInput = typed(py, document)?;
        let core = &self.core;
        py.detach(|| core.memorize_document(input)).map_err(err)
    }

    /// [`Self::memorize_document`] for a list of documents in one transaction.
    fn memorize_documents(
        &self,
        py: Python<'_>,
        documents: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<u32>> {
        let inputs: Vec<DocumentInput> = typed(py, documents)?;
        let core = &self.core;
        py.detach(|| core.memorize_documents(inputs)).map_err(err)
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
        let core = &self.core;
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
        let core = &self.core;
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
        let core = &self.core;
        let hits = py
            .detach(|| core.search_documents_ranked(query, &options, &ranking))
            .map_err(err)?;
        to_py(py, &serde_json::to_value(hits).map_err(err)?)
    }

    fn delete_document(&self, py: Python<'_>, id: &str) -> PyResult<bool> {
        let core = &self.core;
        py.detach(|| core.delete_document(id)).map_err(err)
    }

    fn get_document<'py>(&self, py: Python<'py>, id: &str) -> PyResult<Bound<'py, PyAny>> {
        let core = &self.core;
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

    /// Stores `vector` under `id` in the vector index (the vertex graph).
    fn memorize_vector(&self, py: Python<'_>, id: String, vector: Vec<f32>) -> PyResult<()> {
        let core = &self.core;
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
        let core = &self.core;
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
        let memory = &self.memory;
        let result = py.detach(|| memory.call(op, request)).map_err(err)?;
        to_py(py, &result)
    }
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWarm>()
}
