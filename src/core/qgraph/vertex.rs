// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Graph index with one compact code per vertex instead of one code per edge.
//!
//! Every vertex stores its mean-centred vector as 8-bit integers with one scale (locally
//! adaptive scalar quantization, LVQ; Aguerrebere et al., VLDB 2023). Traversal estimates
//! `<q, x> - <q, mean> ~= s_x * s_q * <c_q, c_x>` with an int8 query code and one integer dot
//! product per neighbour, so a neighbour costs one short code fetch (`dim` bytes, the scales
//! live in their own small array) rather than a full-precision vector. An optional second code
//! of the quantization residual (8-bit, LVQ-8x8, or 4-bit) re-ranks the best `rerank`
//! candidates of the final pool with a float query.
//!
//! Memory per vertex is `4 * degree + dim + 4` bytes, plus `dim + 4` with the 8-bit residual
//! (`dim / 2 + 4` with the 4-bit one), against `4 * dim` for the vectors alone in an HNSW
//! index.

use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use rayon::prelude::*;

use super::adjacency::Adjacency;
use super::fastscan::{pack32, scan32, BLOCK};
use super::kernels::{
    dot_i4_query, dot_i4_query_x4, dot_i8_query, dot_i8_query_x4, interleave_for_i4, prefetch,
    prefetch_bytes, raw_one, QueryCode, Rotation, SplitMix,
};
use super::rerank::{dot_f32_i4, dot_f32_i4_i8, dot_f32_i8};
use super::vnni::query_sum;
use super::{invalid, prepare_rows, upper_bytes, BuildParams, Graph, GraphView, Metric, NONE};
use crate::core::error::WarmError;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static SNAPSHOT_SCRATCH: std::cell::RefCell<Option<SearchScratch>> = const { std::cell::RefCell::new(None) };
}

mod allowed;
mod file;
mod gated;
mod lists;
#[cfg(feature = "experimental")]
mod navigation;
pub use allowed::Allow;
pub use file::Residency;
use file::Slab;
pub use lists::QueryLists;

/// One 128-byte cache line (the line size of Apple cores).
#[repr(C, align(128))]
#[derive(Clone, Copy)]
struct Line([i8; 128]);

/// Bytes starting on a cache-line boundary (zeroed when built here).
struct Lines {
    /// `len.div_ceil(128)` lines.
    lines: Slab<Line>,
    len: usize,
}

impl Lines {
    fn zeroed(len: usize) -> Self {
        let count = len.div_ceil(128);
        let mut lines = Vec::with_capacity(count);
        huge_capacity(lines.as_ptr(), count);
        lines.resize(count, Line([0; 128]));
        Self {
            lines: lines.into(),
            len,
        }
    }

    #[inline(always)]
    fn bytes(&self) -> &[i8] {
        // SAFETY: `Line` is `repr(C)` around `[i8; 128]` (size 128, no padding), so `lines` is
        // `128 * lines.len() >= len` initialized, contiguous bytes.
        unsafe { std::slice::from_raw_parts(self.lines.as_ptr().cast::<i8>(), self.len) }
    }

    fn bytes_mut(&mut self) -> &mut [i8] {
        let len = self.len;
        let lines = self.lines.as_mut_slice();
        // SAFETY: as in `bytes`, with the exclusive borrow of `lines`.
        unsafe { std::slice::from_raw_parts_mut(lines.as_mut_ptr().cast::<i8>(), len) }
    }
}

/// Logical payload bytes counted by [`VGraph::index_bytes`], including row padding.
/// Excludes allocation capacity, slab rounding, object/allocator metadata, rotations and
/// query scratch. These are not resident, unique, proportional, or peak memory measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct VGraphMemoryBreakdown {
    pub adjacency_ids_bytes: usize,
    pub adjacency_offsets_bytes: usize,
    pub traversal_codes_bytes: usize,
    pub traversal_scales_bytes: usize,
    pub residual_codes_bytes: usize,
    pub residual_scales_bytes: usize,
    pub mean_bytes: usize,
    pub norms_bytes: usize,
    pub original_ids_bytes: usize,
    pub screen_bytes: usize,
    pub neighbor_blocks_bytes: usize,
    pub upper_layers_bytes: usize,
}

impl VGraphMemoryBreakdown {
    pub fn total_bytes(&self) -> usize {
        self.adjacency_ids_bytes
            + self.adjacency_offsets_bytes
            + self.traversal_codes_bytes
            + self.traversal_scales_bytes
            + self.residual_codes_bytes
            + self.residual_scales_bytes
            + self.mean_bytes
            + self.norms_bytes
            + self.original_ids_bytes
            + self.screen_bytes
            + self.neighbor_blocks_bytes
            + self.upper_layers_bytes
    }
}

pub struct VGraph {
    n: usize,
    dim: usize,
    metric: VMetric,
    /// `dim` rounded up to a multiple of 16 (32 with 4-bit codes), the integer kernels' width.
    padded: usize,
    /// Bits per traversal code coordinate: 8, or 4 (two per byte).
    bits: u8,
    /// Bytes of one traversal code: `padded` or `padded / 2`.
    cbytes: usize,
    /// Bytes from one code row to the next: `cbytes`, or with aligned rows the smallest
    /// power of two (up to 64) or multiple of 128 that holds it, so no row straddles a line.
    stride: usize,
    degree: usize,
    mean: Vec<f32>,
    /// Euclidean: `|x|^2 / 2` per stored vertex, the norm term of the distance order; empty
    /// for angular.
    half_norms: Slab<f32>,
    /// Neighbour ids, `degree` per vertex padded with `NONE`; empty with 3-byte ids.
    adj: Slab<u32>,
    /// The same as 3-byte little-endian ids (padded with `NONE3`); empty with `u32` ids.
    adj3: Slab<u8>,
    /// In-memory exact-degree spans; empty for the persisted fixed-slot layout.
    adj_off: Vec<u32>,
    /// Delta-coded lists (`id_bytes` 0): per vertex, the sorted neighbour ids as LEB128
    /// varints of the zigzag first difference to the vertex and the gaps less one after,
    /// with `adjv_off[v]..adjv_off[v + 1]` the byte span of vertex `v`; empty otherwise.
    adjv: Slab<u8>,
    adjv_off: Slab<u32>,
    codes: Lines,
    scales: Slab<f32>,
    /// 0 (none), 4 or 8.
    res_bits: u8,
    /// Bytes per residual row: `padded` (8-bit) or `padded / 2` (4-bit, two per byte).
    res_stride: usize,
    residual: Slab<i8>,
    res_scales: Slab<f32>,
    entry: u32,
    upper_ids: Vec<u32>,
    layers: Vec<Vec<Vec<u32>>>,
    /// Original id of each stored vertex when the vertices were renumbered; empty otherwise.
    ids: Slab<u32>,
    /// The stored vertex of each original id, built on the first filtered search; unused when
    /// the vertices were not renumbered.
    inverse: OnceLock<Vec<u32>>,
    /// 1-bit screen rows: `screen_words` u64 of sign bits of the rotated centred vector, then
    /// one u64 holding the f32 factor (low half) and the popcount (high half); empty without
    /// a screen.
    screen: Slab<u64>,
    screen_words: usize,
    screen_rot: Option<Rotation>,
    /// Standard deviation of (screen estimate - 8-bit estimate), in estimate units, measured
    /// at build on index vectors used as queries.
    screen_sigma: f32,
    /// Bits per coordinate of the neighbour-block codes (0 = no blocks; 1, 2 or 4). See
    /// [`VGraph::with_blocks`].
    block_bits: u8,
    /// Coordinates of a block code: the block rotation's width.
    block_dim: usize,
    /// Nibble groups per block code: `block_dim * block_bits / 4`.
    block_m: usize,
    /// Blocks per vertex: `degree.div_ceil(32)`.
    blocks_per_vertex: usize,
    /// Per vertex, `blocks_per_vertex` records of `16 * block_m` code bytes in the fast-scan
    /// layout followed by the 32 neighbours' scales as little-endian f32.
    blocks: Lines,
    block_rot: Option<Rotation>,
    /// Error model of the block estimates: `var = block_kappa * s^2 * |q - x_v|^2` for an
    /// edge of scale `s` seen from the expanded vertex `v`, calibrated at build.
    block_kappa: f32,
    /// Candidate lists learned from construction queries ([`VGraph::set_query_lists`]).
    lists: Option<QueryLists>,
}

mod cache;
mod graph;
#[cfg(feature = "experimental")]
mod graph_query;
mod helpers;
mod hugepages;
mod layout;
mod params;
mod quant;
mod scratch;
mod searcher;
#[cfg(test)]
mod serving_tests;
#[cfg(test)]
mod tests;
use self::helpers::*;
pub use self::hugepages::*;
pub(super) use self::layout::*;
pub use self::params::*;
pub use self::quant::*;
use self::scratch::*;
pub use self::searcher::*;
