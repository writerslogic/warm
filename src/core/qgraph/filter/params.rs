// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

/// Errors of [`FilteredIndex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// An argument has the wrong shape or an out-of-range value.
    Invalid(String),
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "invalid filtered-index argument: {m}"),
        }
    }
}

impl std::error::Error for FilterError {}

/// Build parameters of a [`FilteredIndex`].
#[derive(Debug, Clone, Copy)]
pub struct FilterBuildParams {
    /// Labels with more members than this get an inverted file of their own.
    pub ivf_min: usize,
    /// Target members per list of a label's inverted file.
    pub cluster_size: usize,
    /// Upper bound on the lists of one label.
    pub max_lists: usize,
    /// Labels with at least this many members get a dense membership bitset.
    pub bitset_min: usize,
    /// Lloyd iterations of the per-label k-means.
    pub kmeans_iters: usize,
    /// Training points per list for the per-label k-means (a sample of the members).
    pub train_per_list: usize,
    /// Labels with an inverted file and at least this many members also keep a copy of their
    /// rows grouped by list, so a probe reads memory sequentially (`usize::MAX` = none).
    pub copy_min: usize,
    /// Seed of the k-means sampling.
    pub seed: u64,
    /// Sub-spaces of the 4-bit product quantiser kept for every inverted-file member (0 = none;
    /// otherwise it divides the padded dimension into at most 16 coordinates).
    pub pq_m: usize,
    /// Rows sampled to train the quantiser.
    pub pq_sample: usize,
    /// Lloyd iterations of the quantiser.
    pub pq_iters: usize,
}

impl Default for FilterBuildParams {
    fn default() -> Self {
        Self {
            ivf_min: 20_000,
            cluster_size: 1_000,
            max_lists: 1_024,
            bitset_min: 20_000,
            kmeans_iters: 6,
            train_per_list: 48,
            copy_min: usize::MAX,
            seed: 0x5eed,
            pq_m: 0,
            pq_sample: 100_000,
            pq_iters: 8,
        }
    }
}

/// Search parameters of [`FilteredIndex::search`].
#[derive(Debug, Clone, Copy)]
pub struct FilterSearchParams {
    /// Queries whose rarest label has at most this many members are answered exactly by
    /// scoring every match.
    pub brute_max: usize,
    /// Matches to score before an inverted-file search stops.
    pub target: usize,
    /// Lists an inverted-file search probes at least.
    pub min_lists: usize,
    /// Multi-label queries whose rarest label has at most this many members are intersected
    /// first; when the matches number at most `brute_max` they are all scored exactly.
    pub intersect_max: usize,
    /// Candidates the 4-bit screen of an inverted file keeps for the exact re-rank (0 = no
    /// screen: lists are scored exactly).
    pub rerank: usize,
}

impl Default for FilterSearchParams {
    fn default() -> Self {
        Self {
            brute_max: 20_000,
            target: 5_000,
            min_lists: 1,
            intersect_max: 200_000,
            rerank: 0,
        }
    }
}

/// Execution choices that never change a result: the same ids and distances for any value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterTuning {
    /// [`FilteredIndex::search_batch`] runs queries that share a rarest label back to back on
    /// one thread, so the label's rows and lists stay cached for the next of them.
    pub sort_queries: bool,
    /// Rows ahead of the one being scored whose cache lines are prefetched.
    pub prefetch: usize,
    /// Score with 512-bit `vpdpbusd` where the CPU has AVX512-VNNI.
    pub wide: bool,
    /// Skip the tombstone probe while nothing is deleted.
    pub skip_tombstones: bool,
    /// Test the other labels of a multi-label query a block of ids at a time, with the
    /// bitset words of the next block prefetched, instead of one id at a time.
    pub block_filter: bool,
    /// Keep the matches of the last multi-label query's labels, so a query with the same
    /// labels right after it (see `sort_queries`) skips the intersection.
    pub cache_intersections: bool,
    /// `(target, rerank)` by query class, `[multi-label][band]`, replacing the search
    /// parameters (0 keeps them). Unlike the other fields this changes results: it moves the
    /// recall-time trade-off per class.
    pub class_params: [[(usize, usize); 4]; 2],
    /// Rarest-label sizes at which bands 1, 2 and 3 of `class_params` start.
    pub class_edges: [usize; 3],
}

impl FilterTuning {
    /// Every execution choice off: one query at a time, one id at a time, no caches.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            sort_queries: false,
            prefetch: 24,
            wide: false,
            skip_tombstones: false,
            block_filter: false,
            cache_intersections: false,
            class_params: [[(0, 0); 4]; 2],
            class_edges: [20_000, 50_000, 200_000],
        }
    }
}

impl Default for FilterTuning {
    fn default() -> Self {
        Self {
            sort_queries: true,
            skip_tombstones: true,
            block_filter: true,
            cache_intersections: true,
            ..Self::plain()
        }
    }
}

impl FilterSearchParams {
    /// Parameters under which every query returns the exact filtered top-k.
    #[must_use]
    pub fn exact() -> Self {
        Self {
            brute_max: usize::MAX,
            target: usize::MAX,
            min_lists: usize::MAX,
            intersect_max: usize::MAX,
            rerank: usize::MAX,
        }
    }
}
