// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Scoring contract of the serving codes, independent of graph construction geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VMetric {
    Angular,
    Euclidean,
    InnerProduct,
}

impl From<Metric> for VMetric {
    fn from(metric: Metric) -> Self {
        match metric {
            Metric::Angular => Self::Angular,
            Metric::Euclidean => Self::Euclidean,
        }
    }
}

impl VMetric {
    pub(super) fn code(self) -> u64 {
        match self {
            Self::Angular => 0,
            Self::Euclidean => 1,
            Self::InnerProduct => 2,
        }
    }
}

#[derive(Default, serde::Serialize)]
pub struct QueryProfile {
    pub query_encoding_ns: u64,
    pub entry_ns: u64,
    pub adjacency_visited_prefetch_ns: u64,
    pub scoring_pool_ns: u64,
    pub rerank_sort_ns: u64,
    pub total_ns: u64,
    pub expansions: usize,
    pub neighbors_read: usize,
    pub entry_evaluations: usize,
    pub traversal_evaluations: usize,
    pub rerank_evaluations: usize,
    pub beam_retained: usize,
    pub retained: usize,
}

/// Search parameters of a [`VGraph`].
#[derive(Clone, Copy, Debug)]
pub struct VSearchParams {
    /// Candidate pool size (beam width).
    pub ef: usize,
    /// Independent result capacity, offered every row score before beam admission.
    /// 0 uses the legacy final beam; otherwise bounded to `max(k, reservoir).min(n)`.
    /// Traversal, including the `ef >= k` rule, is unchanged.
    pub reservoir: usize,
    /// Number of best pool candidates re-scored with the float query (and the residual code,
    /// when built); 0 returns the pool order.
    pub rerank: usize,
    /// Early stop: end the traversal once this many consecutive expansions have left the
    /// `k` best estimates unchanged (0 = the pool rule alone). The pool rule still applies.
    pub patience: usize,
    /// With a screen (see [`VGraph::from_graph_with`]): a fresh neighbour gets the 8-bit
    /// estimate only if its 1-bit estimate minus this many standard deviations of the
    /// screen's error could still enter the full pool. Larger values skip less.
    pub screen_sigmas: f32,
    /// Consult the screen when the index has one (false runs the plain search on it).
    pub screen: bool,
    /// With neighbour blocks: fuse the block estimates a vertex receives from every expanded
    /// vertex that lists it (precision-weighted by the edge residual's scale) and re-admit
    /// it to the pool when the fused estimate qualifies, instead of keeping its first one.
    pub fuse: bool,
    /// Per-row search: estimate only the first this many neighbours of each expanded vertex
    /// (lists keep the build's order, nearest first); 0 = all.
    pub neighbour_cap: usize,
}

impl Default for VSearchParams {
    fn default() -> Self {
        Self {
            ef: 64,
            reservoir: 0,
            rerank: 0,
            patience: 0,
            screen_sigmas: 1.0,
            screen: true,
            fuse: false,
            neighbour_cap: 0,
        }
    }
}

/// Runtime knobs of the search loop, each off by default so that the results are those of the
/// plain search. [`VGraph::searcher`] reads them once per process from the environment
/// ([`SearchKnobs::from_env`]); [`VGraph::searcher_with`] takes them explicitly.
///
/// Mechanisms, and what they may change:
/// * `prefetch` (`WARM_QGRAPH_PREFETCH`, 0..=3, results never change): 0 prefetches the next
///   candidate's neighbour list, the code row and estimate terms (scale, Euclidean norm) of
///   every admitted neighbour, and the rows the re-rank reads; 2 adds the neighbour list of
///   the second-nearest unexpanded candidate; 3 adds a lookahead that, after an expansion,
///   prefetches the code rows (or screen rows) of the next candidate's unvisited neighbours.
/// * `batch_est` (`WARM_QGRAPH_BATCH_EST`, results never change): the admitted neighbours'
///   rows are estimated four at a time with shared query loads; the integer sums are the
///   same. The query paths already do this for 8-bit and 4-bit rows, so it only affects
///   searches instantiated without four-row estimates.
/// * `frontier` (`WARM_QGRAPH_FRONTIER`, `0.0..=1.0`, 1.0 = off): with a full pool, stop when
///   the nearest unexpanded estimate is past the point `frontier` of the way from the `k`-th
///   best to the `ef`-th best. Fewer expansions, so lower recall at a given `ef`; the
///   recall/QPS curve over `ef` is what to compare.
/// * `block_screen` (`WARM_QGRAPH_BLOCK_SCREEN`, needs [`VGraph::with_blocks`]): the neighbour
///   block screens instead of ordering; survivors fetch their code row and the pool holds row
///   estimates, as in the plain search. A fresh neighbour is skipped when its block estimate,
///   improved by `screen_sigmas` deviations of the calibrated edge error, could not enter the
///   full pool. Off, the block search is the existing one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchKnobs {
    /// Prefetch depth, 0 to 3 (values above 3 act as 3).
    pub prefetch: u8,
    /// Estimate admitted neighbours four rows at a time.
    pub batch_est: bool,
    /// Frontier stop fraction in `0.0..=1.0`; 1.0 (or a non-finite value) is off.
    pub frontier: f32,
    /// Screen with the neighbour blocks and estimate survivors from their rows.
    pub block_screen: bool,
    /// Bound-gated re-rank: with `z > 0`, candidates are re-scored in estimate order and the
    /// re-rank stops at the first whose estimate, corrected by the mean error and lowered by
    /// `z` standard deviations (both measured on this query's re-scored rows), cannot beat
    /// the current `k`-th exact distance. `rerank` stays the cap. 0 (or non-finite) is off.
    pub rerank_z: f32,
}

impl SearchKnobs {
    /// Every knob off: the plain search.
    pub const OFF: Self = Self {
        prefetch: 0,
        batch_est: false,
        frontier: 1.0,
        block_screen: false,
        rerank_z: 0.0,
    };

    /// The knobs set in the environment, read once per process; unset or invalid values
    /// (reported once) take the [`SearchKnobs::OFF`] value.
    pub fn from_env() -> Self {
        static KNOBS: OnceLock<SearchKnobs> = OnceLock::new();
        *KNOBS.get_or_init(|| Self {
            prefetch: env_knob::<u8>("WARM_QGRAPH_PREFETCH").unwrap_or(0).min(3),
            batch_est: env_knob::<u8>("WARM_QGRAPH_BATCH_EST").unwrap_or(0) != 0,
            frontier: env_knob::<f32>("WARM_QGRAPH_FRONTIER")
                .filter(|f| f.is_finite())
                .map_or(1.0, |f| f.clamp(0.0, 1.0)),
            block_screen: env_knob::<u8>("WARM_QGRAPH_BLOCK_SCREEN").unwrap_or(0) != 0,
            rerank_z: env_knob::<f32>("WARM_QGRAPH_RERANK_Z")
                .filter(|z| z.is_finite() && *z > 0.0)
                .unwrap_or(0.0),
        })
    }
}

impl Default for SearchKnobs {
    fn default() -> Self {
        Self::OFF
    }
}

/// `name` parsed as `T`: `None` when unset, and when the value does not parse, which is
/// reported.
pub(super) fn env_knob<T: std::str::FromStr>(name: &str) -> Option<T> {
    let value = crate::compatibility::var(name).ok()?;
    match value.trim().parse::<T>() {
        Ok(v) => Some(v),
        Err(_) => {
            eprintln!("{name}={value:?} is not a valid value; using the default");
            None
        }
    }
}
