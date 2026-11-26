// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Python surface of the stage 1 in-place mutable base (`core::qgraph::MutableBase`): `MutableIndex`. In memory only, no log and no checkpoint, and
//! independent of `UpdatableIndex`.

use numpy::{
    AllowTypeChange, IntoPyArray, PyArray2, PyArrayLike1, PyArrayLike2, PyUntypedArrayMethods,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use rayon::prelude::*;

use super::vector_index::{thread_pool, u32_ids, update_err};
use crate::core::qgraph::{BuildParams, Metric, MutableBase, MutableOptions, UpdateError};

/// A graph over `max_pts` pre-sized slots that takes inserts and deletes in place. A delete
/// tombstones its slot; a repair scan splices dead slots out of the lists that name them and
/// frees them, and runs when an insert finds too few free slots (or, with `dead_trigger` above
/// zero, after a delete that leaves that share of the allocated slots dead). Ids are
/// caller-chosen integers in 0..2^32-1. Scores come from exact f32 rows.
#[pyclass(name = "MutableIndex")]
struct PyMutableIndex {
    base: MutableBase,
    pool: Option<rayon::ThreadPool>,
    ef: usize,
}

impl PyMutableIndex {
    fn run<R: Send>(&self, f: impl FnOnce() -> R + Send) -> R {
        match &self.pool {
            Some(pool) => pool.install(f),
            None => f(),
        }
    }

    /// The caller's rows, borrowed when they are a C-contiguous float32 matrix.
    fn rows<'a>(
        &self,
        x: &'a PyArrayLike2<'_, f32, AllowTypeChange>,
        n: usize,
    ) -> PyResult<std::borrow::Cow<'a, [f32]>> {
        let (rows, dim) = (x.shape()[0], x.shape()[1]);
        if rows != n || dim != self.base.dim() {
            return Err(PyValueError::new_err(format!(
                "expected a ({n}, {}) matrix, got ({rows}, {dim})",
                self.base.dim()
            )));
        }
        Ok(match x.as_slice() {
            Ok(rows) if x.is_c_contiguous() => std::borrow::Cow::Borrowed(rows),
            _ => std::borrow::Cow::Owned(x.as_array().iter().copied().collect()),
        })
    }
}

#[pymethods]
impl PyMutableIndex {
    /// Creates an empty index of `max_pts` slots (`ceil(max_pts * (1 + slack))`) of `dim`
    /// dimensions. `metric` is "angular" or "euclidean"; `degree` (R, even), `build_ef` (L)
    /// and `alpha` shape the graph as for `UpdatableIndex`. `threads` bounds insert and batch
    /// search parallelism (0 = the global rayon pool). `build_codes` None follows the build
    /// (on from 128 dimensions). `dead_trigger` in [0, 1]: scan after a delete once dead
    /// slots are at least this fraction of the allocated slots; 0 scans only when an insert
    /// needs slots. `repair_ld` must stay 0 (delete-time repair is not implemented).
    #[staticmethod]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (max_pts, dim, metric="euclidean", degree=32, build_ef=100, alpha=1.0,
        threads=0, slack=0.0, dead_trigger=0.0, repair_ld=0, repair_c=3, build_codes=None))]
    fn create(
        py: Python<'_>,
        max_pts: usize,
        dim: usize,
        metric: &str,
        degree: usize,
        build_ef: usize,
        alpha: f32,
        threads: usize,
        slack: f64,
        dead_trigger: f64,
        repair_ld: usize,
        repair_c: usize,
        build_codes: Option<bool>,
    ) -> PyResult<Self> {
        let metric = match metric {
            "angular" => Metric::Angular,
            "euclidean" => Metric::Euclidean,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown metric {other:?}; MutableIndex supports 'angular' and 'euclidean'"
                )))
            }
        };
        if !degree.is_multiple_of(2) {
            return Err(PyValueError::new_err("degree must be even"));
        }
        let params = BuildParams {
            metric,
            degree,
            build_ef,
            alpha,
            ..BuildParams::default()
        };
        let options = MutableOptions {
            slack,
            dead_trigger,
            repair_ld,
            repair_c,
            build_codes,
            ..MutableOptions::new(max_pts)
        };
        let pool = thread_pool(threads)?;
        let base = py
            .detach(|| MutableBase::new(dim, &params, &options))
            .map_err(update_err)?;
        Ok(Self { base, pool, ef: 64 })
    }

    /// Search effort used by `search` when it is not given one.
    fn set_query_arguments(&mut self, ef: usize) -> PyResult<()> {
        if ef == 0 {
            return Err(PyValueError::new_err("ef must be positive"));
        }
        self.ef = ef;
        Ok(())
    }

    /// Adds rows under new `ids` (live ids are refused). `chunk` rows go to the graph per
    /// call into the core (0 = the whole matrix as one batch); each chunk is all or none, the
    /// call as a whole is not. Raises ValueError when the free and dead slots cannot hold
    /// the rows.
    #[pyo3(signature = (ids, x, chunk=0))]
    fn insert(
        &mut self,
        py: Python<'_>,
        ids: PyArrayLike1<'_, i64, AllowTypeChange>,
        x: PyArrayLike2<'_, f32, AllowTypeChange>,
        chunk: usize,
    ) -> PyResult<()> {
        let ids = u32_ids(&ids)?;
        let rows = self.rows(&x, ids.len())?;
        let dim = self.base.dim();
        let step = if chunk == 0 { ids.len().max(1) } else { chunk };
        let (base, pool) = (&mut self.base, &self.pool);
        py.detach(|| {
            let mut go = || -> Result<(), UpdateError> {
                for (ids, rows) in ids.chunks(step).zip(rows.chunks(step * dim)) {
                    base.insert_batch(ids, rows)?;
                }
                Ok(())
            };
            match pool {
                Some(pool) => pool.install(go),
                None => go(),
            }
        })
        .map_err(update_err)
    }

    /// Removes live `ids`, all or none. A scan may run afterwards (see `dead_trigger`).
    fn delete(
        &mut self,
        py: Python<'_>,
        ids: PyArrayLike1<'_, i64, AllowTypeChange>,
    ) -> PyResult<()> {
        let ids = u32_ids(&ids)?;
        let (base, pool) = (&mut self.base, &self.pool);
        py.detach(|| match pool {
            Some(pool) => pool.install(|| base.delete_batch(&ids)),
            None => base.delete_batch(&ids),
        })
        .map_err(update_err)
    }

    /// Runs a repair scan now and returns (repaired lists, freed slots, orphans attached,
    /// entry changed).
    fn repair_scan(&mut self, py: Python<'_>) -> (usize, usize, usize, bool) {
        let (base, pool) = (&mut self.base, &self.pool);
        let s = py.detach(|| match pool {
            Some(pool) => pool.install(|| base.repair_scan()),
            None => base.repair_scan(),
        });
        (s.repaired, s.freed, s.orphans, s.entry_changed)
    }

    /// The `k` best live ids of every row of `x` as `(ids, dists)`: an (n, k) int64 array
    /// nearest first, -1 padded when fewer than `k` ids are live, and the matching float32
    /// array of squared Euclidean distances or angular distances (1 - cosine), inf padded.
    /// `ef` defaults to the value set by `set_query_arguments`.
    #[pyo3(signature = (x, k, ef=None))]
    #[allow(clippy::type_complexity)]
    fn search<'py>(
        &self,
        py: Python<'py>,
        x: PyArrayLike2<'py, f32, AllowTypeChange>,
        k: usize,
        ef: Option<usize>,
    ) -> PyResult<(Bound<'py, PyArray2<i64>>, Bound<'py, PyArray2<f32>>)> {
        if k == 0 {
            return Err(PyValueError::new_err("k must be positive"));
        }
        let ef = ef.unwrap_or(self.ef);
        if ef == 0 {
            return Err(PyValueError::new_err("ef must be positive"));
        }
        let nq = x.shape()[0];
        let queries = self.rows(&x, nq)?;
        let dim = self.base.dim();
        let euclidean = self.base.metric() == Metric::Euclidean;
        // Bound the result buffers before allocating: k is caller-controlled.
        let total = nq
            .checked_mul(k)
            .filter(|&t| t <= 1usize << 31)
            .ok_or_else(|| PyValueError::new_err("k too large for this query batch"))?;
        let mut ids = vec![-1i64; total];
        let mut dists = vec![f32::INFINITY; total];
        py.detach(|| {
            self.run(|| {
                ids.par_chunks_mut(k)
                    .zip(dists.par_chunks_mut(k))
                    .zip(queries.par_chunks(dim))
                    .try_for_each(|((id_row, dist_row), q)| {
                        let qq: f32 = q.iter().map(|v| v * v).sum();
                        let found = self.base.search(q, k, ef)?;
                        for ((id, dist), (score, found_id)) in
                            id_row.iter_mut().zip(dist_row.iter_mut()).zip(found)
                        {
                            *id = i64::from(found_id);
                            *dist = if euclidean {
                                (2.0 * score + qq).max(0.0)
                            } else {
                                1.0 + score / qq.sqrt().max(f32::MIN_POSITIVE)
                            };
                        }
                        Ok::<_, UpdateError>(())
                    })
            })
        })
        .map_err(update_err)?;
        let shape = (nq, k);
        Ok((
            shape_to_py(py, numpy::ndarray::Array2::from_shape_vec(shape, ids))?,
            shape_to_py(py, numpy::ndarray::Array2::from_shape_vec(shape, dists))?,
        ))
    }

    /// Slot accounting and cumulative scan work as a dict: capacity, live, dead, free, scans,
    /// repaired_rows, freed_slots, orphans.
    fn stats(&self) -> std::collections::BTreeMap<&'static str, usize> {
        let s = self.base.stats();
        std::collections::BTreeMap::from([
            ("capacity", s.capacity),
            ("live", s.live),
            ("dead", s.dead),
            ("free", s.free),
            ("scans", s.scans),
            ("repaired_rows", s.repaired_rows),
            ("freed_slots", s.freed_slots),
            ("orphans", s.orphans),
        ])
    }

    /// Whether `id` is live.
    fn contains(&self, id: i64) -> bool {
        u32::try_from(id).is_ok_and(|id| self.base.contains(id))
    }

    #[getter]
    fn dim(&self) -> usize {
        self.base.dim()
    }

    fn __len__(&self) -> usize {
        self.base.len()
    }
}

fn shape_to_py<'py, T: numpy::Element>(
    py: Python<'py>,
    a: Result<numpy::ndarray::Array2<T>, numpy::ndarray::ShapeError>,
) -> PyResult<Bound<'py, PyArray2<T>>> {
    a.map(|a| a.into_pyarray(py))
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyMutableIndex>()
}
