// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Insertion passes a build runs. Every profile searches the same way and prunes the same way;
/// they differ in how many passes run and how wide the construction search is. Measured on
/// SIFT-1M at 8 threads (benchmarks/results/qgraph_build_profiles_gcp_v1.json).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildProfile {
    /// Two passes at `build_ef` capped at 100 and then at `build_ef`, with a bounded
    /// construction search (see `BOUND_RANK`).
    #[default]
    Standard,
    /// Two passes: the first at `build_ef` capped at 200, the second at `build_ef`, each
    /// expanding its whole pool. The slowest build and the highest recall; the graphs of
    /// earlier releases, byte for byte.
    MaxRecall,
    /// Two passes at `build_ef` capped at 100 and 300.
    High,
    /// Two passes at `build_ef` capped at 100 and 200.
    Balanced,
    /// One pass at `build_ef` capped at 100.
    Fast,
}

impl BuildProfile {
    /// The (candidate list size, alpha) of each insertion pass for a build with these parameters.
    pub(super) fn passes(self, params: &BuildParams) -> Vec<(usize, f32)> {
        let cap = |ef: usize| params.build_ef.min(ef);
        match self {
            Self::Standard => vec![(cap(100), 1.0), (params.build_ef, params.alpha)],
            Self::MaxRecall => vec![(cap(FIRST_PASS_EF), 1.0), (params.build_ef, params.alpha)],
            Self::High => vec![(cap(100), 1.0), (cap(300), params.alpha)],
            Self::Balanced => vec![(cap(100), 1.0), (cap(200), params.alpha)],
            Self::Fast => vec![(cap(100), params.alpha)],
        }
    }

    /// `(rank, slack)` of the construction search's expansion bound, if the profile has one.
    pub(super) fn bound(self) -> Option<(usize, f32)> {
        matches!(self, Self::Standard).then_some((BOUND_RANK, BOUND_SLACK))
    }

    /// The profile named `name` (`standard`, `max-recall`, `high`, `balanced`, `fast`), if any.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "standard" => Self::Standard,
            "max-recall" => Self::MaxRecall,
            "high" => Self::High,
            "balanced" => Self::Balanced,
            "fast" => Self::Fast,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MaxRecall => "max-recall",
            Self::High => "high",
            Self::Balanced => "balanced",
            Self::Fast => "fast",
        }
    }
}

/// Distance a graph is built and searched for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Metric {
    /// Cosine: rows are normalized at build, queries need not be.
    #[default]
    Angular,
    /// Squared Euclidean distance on the rows as given. Construction scores
    /// `|a|^2 + |b|^2 - 2 <a, b>` with exact per-row squared norms; the vertex index orders
    /// candidates by `|x|^2 / 2 - <q, x - mean>`, its code estimate of the same order. The edge
    /// index ([`QGraph`]), the screen and the neighbour blocks are angular only.
    Euclidean,
}

impl Metric {
    pub(super) fn code(self) -> u64 {
        match self {
            Self::Angular => 0,
            Self::Euclidean => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BuildParams {
    /// Distance the graph is built for.
    pub metric: Metric,
    /// Candidate list size of the construction-time greedy search (1..=1,048,576; the first
    /// of the two insertion passes uses at most 200).
    pub build_ef: usize,
    /// Out-degree of the bottom layer (positive, even, and at most 65,536).
    pub degree: usize,
    /// Finite pruning parameter of at least 1.0 for the second pass (the first uses 1.0).
    pub alpha: f32,
    /// Length of the 1-bit edge codes; rounded up to a power of two of at least 64 and at
    /// least the dimension. 0 picks the smallest such length.
    pub code_bits: usize,
    pub seed: u64,
    /// Score the construction search's candidates with the 8-bit codes (one SDOT over a
    /// `dim`-byte row) instead of exact float distances, at every dimension; the expanded
    /// set is re-scored exactly before the prune, which stays exact. The cache header does
    /// not record this yet: build with a fresh cache path when it changes.
    pub quantized_search: bool,
    /// Insertion passes of the build; the default is the schedule of earlier releases. A graph
    /// cache header does not record it: build with a fresh cache path when it changes.
    pub profile: BuildProfile,
}

impl Default for BuildParams {
    fn default() -> Self {
        Self {
            metric: Metric::Angular,
            build_ef: 128,
            degree: DEFAULT_DEGREE,
            alpha: 1.0,
            code_bits: 0,
            seed: 0x5EED,
            quantized_search: false,
            profile: BuildProfile::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SearchParams {
    /// Candidate pool size (beam width).
    pub ef: usize,
    /// Maximum exactly scored vertices per query; 0 means no cap.
    pub max_exact: usize,
}
