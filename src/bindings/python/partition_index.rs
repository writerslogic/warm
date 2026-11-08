// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Python surface of the experimental streaming partition tier
//! (`core::qgraph::partition::PartitionTier`, `docs/research/STREAMING-L22.md`): `PartitionIndex`.
//! In memory only. Exact brute force until the first search with `train_at` live rows, which
//! trains the centres and the PQ codebook once; the centres stay fixed after that.

use numpy::{
    AllowTypeChange, IntoPyArray, PyArray2, PyArrayLike1, PyArrayLike2, PyUntypedArrayMethods,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use super::vector_index::{thread_pool, u32_ids};
use crate::core::error::WarmError;
use crate::core::qgraph::partition::{PartitionParams, PartitionTier, SearchProfile};

const MAX_K: usize = 4096;

fn warm_err(e: WarmError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Seconds spent per operation and the summed phases of the post-training searches.
#[derive(Default)]
struct Totals {
    insert_s: f64,
    delete_s: f64,
    search_s: f64,
    train_search_s: f64,
    searches: u64,
    trained_at_search: Option<u64>,
    trained_at_live: usize,
    phases: SearchProfile,
}

/// A partition (IVF) tier over caller-chosen integer ids in 0..2^32-1, Euclidean only: 5,000
/// k-means lists by default, each row in its nearest list and one SOAR spill list as 4-bit
/// PQ residual codes, scanned with LUT16 tables and re-ranked exactly on the f32 rows.
#[pyclass(name = "PartitionIndex")]
struct PyPartitionIndex {
    tier: PartitionTier,
    dim: usize,
    pool: Option<rayon::ThreadPool>,
    totals: Totals,
}

impl PyPartitionIndex {
    fn rows<'a>(
        &self,
        x: &'a PyArrayLike2<'_, f32, AllowTypeChange>,
        n: usize,
    ) -> PyResult<std::borrow::Cow<'a, [f32]>> {
        let dim = self.dim;
        let (rows, d) = (x.shape()[0], x.shape()[1]);
        if rows != n || d != dim {
            return Err(PyValueError::new_err(format!(
                "expected a ({n}, {dim}) matrix, got ({rows}, {d})"
            )));
        }
        Ok(match x.as_slice() {
            Ok(rows) if x.is_c_contiguous() => std::borrow::Cow::Borrowed(rows),
            _ => std::borrow::Cow::Owned(x.as_array().iter().copied().collect()),
        })
    }
}

/// Probe factors by live size from the held-out static-state curves (STREAMING-L22.md, section 10):
/// small sets have small lists, so they take more probes per query.
const DEFAULT_NPROBE_BY_LIVE: [(usize, f32); 6] = [
    (2_000_000, 1.4),
    (3_000_000, 1.3),
    (4_500_000, 1.0),
    (6_000_000, 0.8),
    (8_000_000, 0.7),
    (10_000_000, 0.6),
];

#[pymethods]
impl PyPartitionIndex {
    /// Creates an empty tier of `dim` dimensions (`pq_m` must divide it). `threads` bounds
    /// training, assignment and search parallelism (0 = the global rayon pool).
    #[staticmethod]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (dim, lists=5000, train_at=1_000_000, nprobe=250, reorder=317, pq_m=50,
        threads=0, seed=7, retrain_at=None, retrain_imb=1.7, retrain_min_inserts=4_000_000,
        soar_lambda=1.0, train_sample=1_000_000, kmeans_iters=10, retrain_warm_iters=3,
        retrain_sample=1_000_000, kmeans_rebalance=true, nprobe_by_live=None))]
    fn create(
        dim: usize,
        lists: usize,
        train_at: usize,
        nprobe: usize,
        reorder: usize,
        pq_m: usize,
        threads: usize,
        seed: u64,
        retrain_at: Option<Vec<usize>>,
        retrain_imb: f32,
        retrain_min_inserts: usize,
        soar_lambda: f32,
        train_sample: usize,
        kmeans_iters: usize,
        retrain_warm_iters: usize,
        retrain_sample: usize,
        kmeans_rebalance: bool,
        nprobe_by_live: Option<Vec<(usize, f32)>>,
    ) -> PyResult<Self> {
        if nprobe == 0 || reorder == 0 {
            return Err(PyValueError::new_err("nprobe and reorder must be positive"));
        }
        let params = PartitionParams {
            lists,
            train_at,
            nprobe,
            reorder,
            pq_m,
            seed,
            retrain_at: retrain_at.unwrap_or_default(),
            retrain_imb,
            retrain_min_inserts,
            soar_lambda,
            train_sample,
            kmeans_iters,
            retrain_warm_iters,
            retrain_sample,
            kmeans_rebalance,
            nprobe_by_live: nprobe_by_live.unwrap_or_else(|| DEFAULT_NPROBE_BY_LIVE.to_vec()),
            ..PartitionParams::default()
        };
        Ok(Self {
            tier: PartitionTier::new(dim, params).map_err(warm_err)?,
            dim,
            pool: thread_pool(threads)?,
            totals: Totals::default(),
        })
    }

    /// Probed lists for later searches.
    fn set_nprobe(&mut self, nprobe: usize) -> PyResult<()> {
        if nprobe == 0 {
            return Err(PyValueError::new_err("nprobe must be positive"));
        }
        self.tier.set_nprobe(nprobe);
        Ok(())
    }

    /// Adds rows under new ids; a live id is refused and nothing is added.
    fn insert(
        &mut self,
        py: Python<'_>,
        ids: PyArrayLike1<'_, i64, AllowTypeChange>,
        x: PyArrayLike2<'_, f32, AllowTypeChange>,
    ) -> PyResult<()> {
        let ids = u32_ids(&ids)?;
        let rows = self.rows(&x, ids.len())?;
        let (tier, pool) = (&mut self.tier, &self.pool);
        let t0 = std::time::Instant::now();
        let res = py.detach(|| match pool {
            Some(p) => p.install(|| tier.insert(&ids, &rows)),
            None => tier.insert(&ids, &rows),
        });
        self.totals.insert_s += t0.elapsed().as_secs_f64();
        res.map_err(warm_err)
    }

    /// Removes ids; unknown ids are ignored. Returns how many were removed.
    fn delete(&mut self, ids: PyArrayLike1<'_, i64, AllowTypeChange>) -> PyResult<usize> {
        let ids = u32_ids(&ids)?;
        let t0 = std::time::Instant::now();
        let n = self.tier.delete(&ids);
        self.totals.delete_s += t0.elapsed().as_secs_f64();
        Ok(n)
    }

    /// The `k` nearest live ids of every row of `x`, nearest first, as an (n, k) int64
    /// array; -1 pads rows when fewer than `k` are found.
    fn search<'py>(
        &mut self,
        py: Python<'py>,
        x: PyArrayLike2<'py, f32, AllowTypeChange>,
        k: usize,
    ) -> PyResult<Bound<'py, PyArray2<i64>>> {
        if k == 0 || k > MAX_K {
            return Err(PyValueError::new_err(format!("k must be in 1..={MAX_K}")));
        }
        let nq = x.shape()[0];
        let queries = self.rows(&x, nq)?;
        let was_trained = self.tier.is_trained();
        let (tier, pool) = (&mut self.tier, &self.pool);
        let t0 = std::time::Instant::now();
        let found = py
            .detach(|| match pool {
                Some(p) => p.install(|| tier.search(&queries, k)),
                None => tier.search(&queries, k),
            })
            .map_err(warm_err)?;
        let wall = t0.elapsed().as_secs_f64();
        let t = &mut self.totals;
        t.searches += 1;
        t.search_s += wall;
        if self.tier.is_trained() {
            let p = self.tier.last_search_profile();
            if !was_trained {
                t.trained_at_search = Some(t.searches);
                t.trained_at_live = self.tier.len();
                t.train_search_s = wall;
            }
            let s = &mut t.phases;
            s.centres_s += p.centres_s;
            s.route_s += p.route_s;
            s.scan_s += p.scan_s;
            s.scan_mean_s += p.scan_mean_s;
            s.lut_mean_s += p.lut_mean_s;
            s.blocks += p.blocks;
            s.admitted += p.admitted;
            s.reorder_s += p.reorder_s;
            s.total_s += p.total_s;
        }
        let mut out = vec![-1i64; nq * k];
        for (row, ids) in out.chunks_mut(k).zip(&found) {
            for (slot, &id) in row.iter_mut().zip(ids) {
                *slot = i64::from(id);
            }
        }
        numpy::ndarray::Array2::from_shape_vec((nq, k), out)
            .map(|a| a.into_pyarray(py))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// The trained centres as a (lists, dim) float32 array (None before training).
    fn centres<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyArray2<f32>>>> {
        let Some(c) = self.tier.centres() else {
            return Ok(None);
        };
        numpy::ndarray::Array2::from_shape_vec((c.len() / self.dim, self.dim), c.to_vec())
            .map(|a| Some(a.into_pyarray(py)))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// (primary, spill) list of each id as an (n, 2) uint32 array; 4294967295 marks a missing
    /// id or an untrained index.
    fn lists_of<'py>(
        &self,
        py: Python<'py>,
        ids: PyArrayLike1<'_, i64, AllowTypeChange>,
    ) -> PyResult<Bound<'py, PyArray2<u32>>> {
        let ids = u32_ids(&ids)?;
        let mut out = Vec::with_capacity(ids.len() * 2);
        for id in &ids {
            out.extend(self.tier.lists_of(*id).unwrap_or([u32::MAX; 2]));
        }
        numpy::ndarray::Array2::from_shape_vec((ids.len(), 2), out)
            .map(|a| a.into_pyarray(py))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Live rows, training state, and the summed operation seconds and search phases.
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let t = &self.totals;
        d.set_item("live", self.tier.len())?;
        d.set_item("trained", self.tier.is_trained())?;
        d.set_item("trained_at_search", t.trained_at_search)?;
        d.set_item("trained_at_live", t.trained_at_live)?;
        d.set_item("train_search_s", t.train_search_s)?;
        d.set_item("searches", t.searches)?;
        d.set_item("insert_s", t.insert_s)?;
        d.set_item("delete_s", t.delete_s)?;
        d.set_item("search_s", t.search_s)?;
        let p = &t.phases;
        d.set_item("centres_s", p.centres_s)?;
        d.set_item("route_s", p.route_s)?;
        d.set_item("scan_s", p.scan_s)?;
        d.set_item("scan_mean_s", p.scan_mean_s)?;
        d.set_item("lut_mean_s", p.lut_mean_s)?;
        d.set_item("blocks", p.blocks)?;
        d.set_item("admitted", p.admitted)?;
        d.set_item("reorder_s", p.reorder_s)?;
        d.set_item("partition_search_s", p.total_s)?;
        Ok(d)
    }

    fn __len__(&self) -> usize {
        self.tier.len()
    }
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPartitionIndex>()
}
