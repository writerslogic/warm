// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Quantized graph index for approximate nearest-neighbour search on angular data, and (the
//! [`Graph`] and [`VGraph`] only, see [`Metric`]) on Euclidean data.
//!
//! Vectors are normalized and rotated (random sign flips and fast Walsh-Hadamard transforms),
//! and a Vamana-style graph with alpha pruning and a fixed out-degree is built on exact inner
//! products, inserting in deterministic batches. Each vertex owns one contiguous block holding
//! its rotated vector, its neighbours' ids, and for every edge a 1-bit RaBitQ code of the edge
//! residual (neighbour minus vertex) with per-edge correction factors.
//!
//! Search keeps a pool of `ef` candidates ordered by estimated distance. Expanding a vertex
//! scores it exactly (this is the re-rank) and estimates all of its neighbours in one batch from
//! the edge codes against a 4-bit query code. The result is the top-k of the exactly scored
//! vertices. `max_exact` caps the number of expansions (0 = no cap).
//!
//! Estimator (RaBitQ, Gao & Long 2024, applied per edge as in SymphonyQG, Gou et al. 2025):
//! with `w = u - v` and `s = sign(w)`, `<q, u> ~= <q, v> + <v, w> + M (<s, q> - <s, v>)` where
//! `M = |w|^2 / |w|_1`. `<q, v>` is exact at expansion time; `<s, q>` comes from the bit planes.
//!
//! [`VGraph`] is the compact alternative on the same [`Graph`]: one 8-bit code per vertex instead
//! of one 1-bit code per edge (see `vertex.rs`).

mod adjacency;
mod build_kernels;
pub(crate) mod centroid;
pub mod connectivity;
#[cfg(feature = "experimental")]
mod exact_l2;
mod fastscan;
pub mod filter;
pub(crate) mod kernels;
mod projection;
mod repair;
mod rerank;
#[cfg(not(target_arch = "wasm32"))]
mod update;
mod vertex;
mod vnni;
#[cfg(not(target_arch = "wasm32"))]
pub use repair::{RepairParams, RepairStats};
pub use update::{
    Consolidation, ConsolidationMode, ConsolidationPolicy, MutableBase, MutableOptions,
    MutableStats, RowStore, ScanStats, UpdatableVGraph, UpdateError,
};

#[cfg(feature = "experimental")]
pub mod diagnostics;

#[cfg(feature = "experimental")]
pub mod projection_diagnostic;

#[cfg(feature = "experimental")]
pub mod order_diagnostic;

#[cfg(feature = "experimental")]
pub mod ivf;

#[cfg(feature = "experimental")]
pub mod partition;

use std::cell::RefCell;

use rayon::prelude::*;

use crate::core::error::WarmError;
use adjacency::Adjacency;

use kernels::{
    as_f32, as_f32_mut, as_u32, as_u32_mut, dot, dot_i8_query, dot_i8_query_x4, QueryCode,
    Rotation, SplitMix,
};
#[cfg(feature = "experimental")]
pub use vertex::QueryProfile;
pub use vertex::{
    huge_page_status, Allow, HugePageStatus, QueryLists, Residency, SearchKnobs, VGraph,
    VGraphMemoryBreakdown, VMetric, VSearchParams, VSearcher,
};

/// Default out-degree of every vertex; one batch of edge codes per expansion.
pub const DEFAULT_DEGREE: usize = 32;
const NONE: u32 = u32::MAX;
/// Smallest batch whose points are run in locality order, and the number of cells that order
/// groups the rows into (a quarter of the square root of the row count, within the bounds).
const LOCALITY_MIN_BATCH: usize = 1024;
const LOCALITY_CELLS: std::ops::RangeInclusive<usize> = 64..=256;

/// Every failure of a build or search names its cause as an invalid parameter.
fn invalid(details: String) -> WarmError {
    WarmError::InvalidParam { details }
}
const UPPER_DEGREE: usize = 16;
const LAYER_RATIO: usize = 16;
const MIN_LAYER: usize = 64;
/// Smallest dimension at which construction screens exact distances with [`BuildCodes`]: a
/// skipped distance then saves a row of at least 512 bytes, which pays for the extra code
/// access.
const BUILD_CODES_MIN_DIM: usize = 128;
/// Largest candidate list of the first construction pass, which only has to make the graph
/// navigable for the second pass (see `benchmarks/results/qgraph_build_schedule_wl02_v1.json`).
const FIRST_PASS_EF: usize = 200;

/// Candidate expansions of the standard profile's construction search. The search stops
/// expanding once the best unexpanded candidate is farther than `BOUND_SLACK` times the
/// `BOUND_RANK`-th best candidate in the pool (squared distances): the pool beyond that rank is
/// kept to fill the lists, not to find the neighbours. Measured on glove-100 (distances there
/// concentrate, so the unbounded search expands most of a 500-entry pool) and SIFT-1M.
const BOUND_RANK: usize = 64;
const BOUND_SLACK: f32 = 1.02;

pub struct QGraph {
    n: usize,
    dim: usize,
    degree: usize,
    words: usize,
    rotation: Rotation,
    stride: usize,
    codes_off: usize,
    factors_off: usize,
    ids_off: usize,
    blocks: Vec<u64>,
    entry: u32,
    /// Global ids of the vertices in the upper layers, by local id.
    upper_ids: Vec<u32>,
    /// Upper-layer adjacency in local ids, largest layer first.
    layers: Vec<Vec<Vec<u32>>>,
}

/// A built proximity graph, before any encoding: over normalized vectors (angular) or the
/// vectors as given (Euclidean). Holds a full-precision copy of the data, so it is a
/// build-time object; the indexes keep only their encodings.
pub struct Graph {
    n: usize,
    dim: usize,
    degree: usize,
    metric: Metric,
    /// Unit rows (angular) or the rows as given (Euclidean).
    vectors: Vec<f32>,
    /// Squared norm of every row (Euclidean); empty for angular.
    norms: Vec<f32>,
    adj: Adjacency,
    entry: u32,
    upper_ids: Vec<u32>,
    layers: Vec<Vec<Vec<u32>>>,
    /// The parameters the graph was built with; the on-disk cache records and checks them.
    build: BuildParams,
    /// Changed by [`Graph::apply_query_repair`]: the build parameters no longer describe it.
    repaired: bool,
}

struct GraphStructure {
    adj: Adjacency,
    entry: u32,
    upper_ids: Vec<u32>,
    layers: Vec<Vec<Vec<u32>>>,
}

mod build_rows;
mod construct;
mod graph;
mod params;
mod prepared;
mod prof;
mod search;
#[cfg(test)]
mod tests;
mod visited;

/// Flushes `temp` to disk, then renames it over `path`.
fn persist_synced(temp: tempfile::NamedTempFile, path: &std::path::Path) -> std::io::Result<()> {
    temp.as_file().sync_all()?;
    crate::core::durable_file::sync_mapped_file(temp.as_file())?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

use self::build_rows::*;
use self::construct::*;
use self::graph::*;
pub use self::params::*;
pub(crate) use self::prepared::*;
pub use self::search::*;
#[cfg(feature = "experimental")]
pub use self::visited::set_build_phase_hook;
use self::visited::*;
