// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Label-filtered exact-metric nearest-neighbour search over 8-bit vectors (Euclidean).
//!
//! Every point carries a set of labels; a query carries one or more labels and asks for the
//! `k` nearest points that hold *all* of them (AND semantics). Each label keeps its sorted
//! member ids; labels with many members also keep a dense membership bitset and an inverted
//! file of their own members (k-means lists trained on that label alone). A query is routed by
//! the posting size of its rarest label:
//!
//! - rarest posting at most `brute_max`: its members are intersected with the other labels and
//!   every match is scored exactly;
//! - otherwise the rarest label's lists are probed nearest-centroid first, members failing the
//!   other labels are skipped, and probing stops once `target` matches have been scored (and at
//!   least `min_lists` lists probed). With `target >= n` every list is probed and the result is
//!   the exact filtered top-k.
//!
//! Distances are exact integers: rows are stored as `x - 128` in i8, the query likewise, and
//! `|q - x|^2 = |q'|^2 + |x'|^2 - 2 q'.x'`, ranked by `|x'|^2 - 2 q'.x'` from the integer dot
//! kernels (VNNI where the CPU has it). Ties break on the smaller id.
//!
//! Deletes set a tombstone that every path checks; inserting a point needs a rebuild.

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

use super::kernels::{dot_i8_query, dot_i8_query_x4, dot_i8_query_x4_wide, prefetch_bytes};

mod fs;
use fs::{FsScratch, Pq};

/// Largest row whose squared u8 Euclidean distance fits the returned `u32`.
pub const MAX_DIMENSIONS: usize = (u32::MAX / (255 * 255)) as usize;
/// Dense label tables are bounded independently of the number of input rows.
pub const MAX_LABELS: usize = 1 << 20;

fn allocation<T: Clone>(length: usize, value: T) -> Result<Vec<T>, FilterError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| FilterError::Invalid("filtered index allocation exceeds capacity".into()))?;
    values.resize(length, value);
    Ok(values)
}

fn product(left: usize, right: usize) -> Result<usize, FilterError> {
    left.checked_mul(right)
        .ok_or_else(|| FilterError::Invalid("filtered index shape exceeds capacity".into()))
}

/// The inverted file of one label: list centroids and the members of each list.
struct LabelIvf {
    /// `lists * dim` centroids in the shifted i8 domain.
    centroids: Vec<i8>,
    /// `|c'|^2` per list.
    centroid_norms: Vec<i32>,
    /// `lists + 1` offsets into `members`.
    offsets: Vec<u32>,
    /// Member ids grouped by list, ascending within a list.
    members: Vec<u32>,
    /// The members' rows in `members` order (`stride` each), or empty.
    rows: Vec<i8>,
    /// `|x'|^2` in `members` order, or empty.
    norms: Vec<i32>,
    /// 4-bit codes in 32-code blocks (empty without a quantiser); `members` is then empty.
    codes: Vec<u8>,
    /// Member ids per block, `u32::MAX` in the padding of a list's last block.
    bids: Vec<u32>,
    /// `lists + 1` first-block indices.
    blocks: Vec<u32>,
}

/// Which path a query took, counted for diagnostics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FilterStats {
    /// Queries scored exactly over their matches.
    pub brute: u64,
    /// Queries answered by a label inverted file.
    pub ivf: u64,
    /// Rows scored.
    pub scored: u64,
}

/// A label-filtered index over 8-bit Euclidean vectors. See the module documentation.
pub struct FilteredIndex {
    dim: usize,
    /// `dim` rounded up to a multiple of 16: the row and centroid width.
    stride: usize,
    n: usize,
    /// Rows as `x - 128`, `n * dim`.
    rows: Vec<i8>,
    /// `|x'|^2` per row.
    norms: Vec<i32>,
    /// `labels + 1` offsets into `postings`.
    label_offsets: Vec<u64>,
    /// Member ids of every label, ascending.
    postings: Vec<u32>,
    /// Bitset slot per label (`u32::MAX` = none).
    bitset_slot: Vec<u32>,
    /// `words` words per slot.
    bitsets: Vec<u64>,
    /// Inverted file per label (`u32::MAX` = none), index into `ivfs`.
    ivf_slot: Vec<u32>,
    ivfs: Vec<LabelIvf>,
    pq: Option<Pq>,
    deleted: Vec<AtomicU64>,
    live: usize,
    tuning: FilterTuning,
    brute_queries: AtomicU64,
    ivf_queries: AtomicU64,
    scored: AtomicU64,
}

mod index;
mod params;
mod scratch;
#[cfg(test)]
mod tests;
mod train;
pub use self::params::*;
use self::scratch::*;
use self::train::*;
