// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Inner-product top-k over learned sparse vectors (SPLADE-style), with a blocked inverted
//! index and a forward index.
//!
//! The layout follows the blocked inverted index of SEISMIC (Bruch et al., arXiv 2404.18812):
//!
//! - every coordinate keeps its `postings_per_term` largest postings (static pruning);
//! - each kept list is split into blocks by assigning every document to the most similar of a
//!   random sample of the list's documents (shallow clustering);
//! - every list stores its block summaries column-wise: for the `summary_coords` coordinates
//!   with the largest summed block maxima, one byte per block (the block's coordinate-wise
//!   maximum, quantized upward against a per-column scale), so the bounds of all of a list's
//!   blocks come out of a few vectorized passes over contiguous bytes;
//! - a forward index holds every document in full, so a candidate's score is its exact inner
//!   product with the query.
//!
//! A search walks the lists of the `query_cut` heaviest query coordinates and scores a block's
//! documents only when the summary's inner product with the query, times `heap_factor`, reaches
//! the current k-th best score.
//!
//! With [`SparseBuildParams::exact`] and [`SparseSearchParams::exact`] (no static pruning, full
//! summaries rounded up, every query coordinate, `heap_factor` 1) a summary bounds the score of
//! every document in its block, so a skipped block cannot hold a result and the search returns
//! the exact top-k, ties broken by the lower document id. That needs non-negative document
//! values, which the build enforces; negative query weights only lower a score, so the bound
//! uses the positive part of the query.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rayon::prelude::*;

/// Largest supported coordinate count: coordinate ids are stored in two bytes.
pub const MAX_DIMS: usize = 1 << 16;

/// Errors from building a [`SparseImpactIndex`] or validating a query.
#[derive(Debug, Clone, PartialEq)]
pub enum SparseIndexError {
    /// The CSR arrays are inconsistent (lengths, offsets or an out-of-range coordinate).
    Malformed(String),
    /// A document value is negative or not finite.
    BadValue { doc: usize, value: f32 },
    /// More coordinates or documents than the format holds.
    TooLarge(String),
}

impl std::fmt::Display for SparseIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "malformed sparse matrix: {m}"),
            Self::BadValue { doc, value } => {
                write!(
                    f,
                    "document {doc} has value {value}; values must be finite and >= 0"
                )
            }
            Self::TooLarge(m) => write!(f, "sparse matrix too large: {m}"),
        }
    }
}

impl std::error::Error for SparseIndexError {}

/// A borrowed CSR matrix: row `r` holds `indices[indptr[r]..indptr[r + 1]]` with `data` values.
#[derive(Clone, Copy)]
pub struct CsrView<'a> {
    pub indptr: &'a [i64],
    pub indices: &'a [i32],
    pub data: &'a [f32],
    pub n_dims: usize,
}

impl<'a> CsrView<'a> {
    /// Reads the big-ann-benchmarks `spmat` layout: int64 rows, cols and nnz, then int64
    /// indptr, int32 indices and float32 data. `bytes` must be 8-byte aligned (a memory map is).
    pub fn from_spmat(bytes: &'a [u8]) -> Result<Self, SparseIndexError> {
        let bad = |m: String| SparseIndexError::Malformed(m);
        let header: &[i64] = bytes
            .get(..24)
            .and_then(|h| bytemuck::try_cast_slice(h).ok())
            .ok_or_else(|| bad("spmat header is truncated or misaligned".into()))?;
        let size = |v: i64| usize::try_from(v).map_err(|_| bad(format!("negative size {v}")));
        let (rows, cols, nnz) = (size(header[0])?, size(header[1])?, size(header[2])?);
        let span = |n: usize, w: usize| {
            n.checked_mul(w)
                .ok_or_else(|| bad("spmat size overflows".into()))
        };
        let indptr_end = 24 + span(rows + 1, 8)?;
        let indices_end = indptr_end + span(nnz, 4)?;
        let data_end = indices_end + span(nnz, 4)?;
        if bytes.len() != data_end {
            return Err(bad(format!(
                "file is {} bytes, header implies {data_end}",
                bytes.len()
            )));
        }
        let cast_err = |e: bytemuck::PodCastError| bad(format!("misaligned section: {e:?}"));
        Ok(Self {
            indptr: bytemuck::try_cast_slice(&bytes[24..indptr_end]).map_err(cast_err)?,
            indices: bytemuck::try_cast_slice(&bytes[indptr_end..indices_end]).map_err(cast_err)?,
            data: bytemuck::try_cast_slice(&bytes[indices_end..data_end]).map_err(cast_err)?,
            n_dims: cols,
        })
    }

    /// Checks lengths, monotone offsets and coordinate range.
    pub fn validate(&self) -> Result<usize, SparseIndexError> {
        let rows = self
            .indptr
            .len()
            .checked_sub(1)
            .ok_or_else(|| SparseIndexError::Malformed("indptr is empty".into()))?;
        if self.indices.len() != self.data.len() {
            return Err(SparseIndexError::Malformed(format!(
                "{} indices but {} values",
                self.indices.len(),
                self.data.len()
            )));
        }
        if self.indptr[0] != 0 || self.indptr[rows] != self.indices.len() as i64 {
            return Err(SparseIndexError::Malformed(
                "indptr must start at 0 and end at nnz".into(),
            ));
        }
        if self.indptr.windows(2).any(|w| w[0] > w[1]) {
            return Err(SparseIndexError::Malformed("indptr decreases".into()));
        }
        if let Some(&bad) = self
            .indices
            .iter()
            .find(|&&c| c < 0 || c as usize >= self.n_dims)
        {
            return Err(SparseIndexError::Malformed(format!(
                "coordinate {bad} outside [0, {})",
                self.n_dims
            )));
        }
        Ok(rows)
    }

    /// Coordinates and values of row `r`.
    pub fn row(&self, r: usize) -> (&'a [i32], &'a [f32]) {
        let (a, b) = (self.indptr[r] as usize, self.indptr[r + 1] as usize);
        (&self.indices[a..b], &self.data[a..b])
    }
}

/// Build-time parameters.
#[derive(Clone, Copy, Debug)]
pub struct SparseBuildParams {
    /// Postings kept per coordinate (the largest values); `usize::MAX` keeps every posting.
    pub postings_per_term: usize,
    /// Blocks per kept list as a fraction of its length (at least one block).
    pub block_fraction: f32,
    /// Summary columns kept per list (the coordinates with the largest summed block maxima);
    /// `usize::MAX` keeps every coordinate.
    pub summary_coords: usize,
    /// Postings transposed at once during the build (bounds the build's working memory).
    pub transpose_budget: usize,
    /// Seed of the centroid sample.
    pub seed: u64,
}

impl SparseBuildParams {
    /// Parameters under which a search with [`SparseSearchParams::exact`] is exact.
    pub fn exact() -> Self {
        Self {
            postings_per_term: usize::MAX,
            summary_coords: usize::MAX,
            ..Self::default()
        }
    }
}

impl Default for SparseBuildParams {
    fn default() -> Self {
        Self {
            postings_per_term: 4000,
            block_fraction: 0.1,
            summary_coords: 192,
            transpose_budget: 1 << 26,
            seed: 0x5eed,
        }
    }
}

/// Query-time parameters.
#[derive(Clone, Copy, Debug)]
pub struct SparseSearchParams {
    /// Query coordinates (heaviest first) whose lists are walked.
    pub query_cut: usize,
    /// A block is scored when `summary score * heap_factor` reaches the k-th best score.
    pub heap_factor: f32,
    /// Share of the bound's slack for coordinates a list keeps no column for: the largest
    /// norm of a block document outside the columns times the norm of the query outside them
    /// (Cauchy-Schwarz), times this factor. 1 keeps the bound an upper bound; 0 ignores those
    /// coordinates.
    pub residual: f32,
}

impl SparseSearchParams {
    pub fn exact() -> Self {
        Self {
            query_cut: usize::MAX,
            heap_factor: 1.0,
            residual: 1.0,
        }
    }
}

/// Blocked inverted index with summaries plus a forward index.
pub struct SparseImpactIndex {
    n_dims: usize,
    n_docs: usize,
    /// Forward index: document `d` is the byte row `fwd[fwd_offsets[d]..fwd_offsets[d + 1]]`:
    /// a little-endian `f32` scale, `n` little-endian `u16` coordinates, then `n` `u8` codes,
    /// so a row is `4 + 3n` bytes and a value is `code * scale` (absolute error at most
    /// half a step, `max / 510`, and a stored value is never zero).
    fwd_offsets: Vec<u64>,
    fwd: Vec<u8>,
    /// Blocks of coordinate `t`: `term_blocks[t]..term_blocks[t + 1]`.
    term_blocks: Vec<u32>,
    /// Documents of block `b`: `block_docs[block_doc_offsets[b]..block_doc_offsets[b + 1]]`.
    block_doc_offsets: Vec<u64>,
    block_docs: Vec<u32>,
    /// Summary columns of coordinate `t`'s list: `col_ids/col_scales[term_cols[t]..term_cols[t + 1]]`
    /// (ids ascending). Column `j` holds one code per block, at
    /// `col_codes[term_code_off[t] + (j - term_cols[t]) * stride ..][..blocks]` with
    /// `stride = pad(blocks)`; a block's maximum of that coordinate is at most `code * scale`.
    term_cols: Vec<u64>,
    n_blocks: usize,
    col_ids: Vec<u16>,
    col_scales: Vec<f32>,
    /// Scale of the residual column of each coordinate's list: the extra column after its
    /// summary columns holds, per block, the largest norm of a document's part outside the columns.
    res_scales: Vec<f32>,
    term_code_off: Vec<u64>,
    col_codes: Vec<u8>,
}

/// Column length for `blocks` blocks: a multiple of 32 so vector passes need no tail.
#[inline]
fn pad(blocks: usize) -> usize {
    blocks.div_ceil(32) * 32
}

/// One kept list after clustering: its blocks' documents and summaries.
struct TermBlocks {
    doc_counts: Vec<u32>,
    docs: Vec<u32>,
    col_ids: Vec<u16>,
    col_scales: Vec<f32>,
    res_scale: f32,
    codes: Vec<u8>,
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Per-thread scratch for the build: dense maxima and a coordinate-to-centroid map.
struct BuildScratch {
    dense: Vec<f32>,
    touched: Vec<u16>,
    starts: Vec<u32>,
}

impl BuildScratch {
    fn new(n_dims: usize) -> Self {
        Self {
            dense: vec![0.0; n_dims],
            touched: Vec::new(),
            starts: vec![u32::MAX; n_dims + 1],
        }
    }
}

impl SparseImpactIndex {
    /// Builds the index over the rows of `csr`. Zero values are dropped.
    pub fn build(csr: CsrView<'_>, params: SparseBuildParams) -> Result<Self, SparseIndexError> {
        let n_docs = csr.validate()?;
        if csr.n_dims > MAX_DIMS {
            return Err(SparseIndexError::TooLarge(format!(
                "{} coordinates, at most {MAX_DIMS}",
                csr.n_dims
            )));
        }
        if n_docs > u32::MAX as usize {
            return Err(SparseIndexError::TooLarge(format!("{n_docs} documents")));
        }
        if let Some(i) = csr.data.iter().position(|v| !(v.is_finite() && *v >= 0.0)) {
            let doc = csr.indptr.partition_point(|&p| p <= i as i64) - 1;
            return Err(SparseIndexError::BadValue {
                doc,
                value: csr.data[i],
            });
        }
        let n_dims = csr.n_dims;

        // Forward index (rows keep their order; zeros dropped).
        let mut fwd_offsets = Vec::with_capacity(n_docs + 1);
        fwd_offsets.push(0u64);
        let nnz = csr.data.iter().filter(|&&v| v > 0.0).count();
        let mut fwd: Vec<u8> = Vec::with_capacity(nnz * 3 + 4 * n_docs);
        advise_huge_pages(&fwd);
        let mut df = vec![0u64; n_dims];
        for r in 0..n_docs {
            let (ids, vals) = csr.row(r);
            let scale = vals.iter().fold(0f32, |m, &v| m.max(v)) / 255.0;
            fwd.extend_from_slice(&scale.to_le_bytes());
            for (&c, &v) in ids.iter().zip(vals) {
                if v > 0.0 {
                    fwd.extend_from_slice(&(c as u16).to_le_bytes());
                    df[c as usize] += 1;
                }
            }
            for &v in vals {
                if v > 0.0 {
                    fwd.push(((v / scale).round().clamp(1.0, 255.0)) as u8);
                }
            }
            fwd_offsets.push(fwd.len() as u64);
        }

        let mut index = Self {
            n_dims,
            n_docs,
            fwd_offsets,
            fwd,
            term_blocks: Vec::with_capacity(n_dims + 1),
            block_doc_offsets: vec![0],
            block_docs: Vec::new(),
            term_cols: vec![0],
            col_ids: Vec::new(),
            col_scales: Vec::new(),
            res_scales: Vec::new(),
            term_code_off: vec![0],
            col_codes: Vec::new(),
            n_blocks: 0,
        };
        index.term_blocks.push(0);

        // Transpose a range of coordinates at a time, then cluster each list in parallel.
        let budget = params.transpose_budget.max(1) as u64;
        let mut lo = 0usize;
        while lo < n_dims {
            let mut hi = lo;
            let mut total = 0u64;
            while hi < n_dims && (hi == lo || total + df[hi] <= budget) {
                total += df[hi];
                hi += 1;
            }
            let mut starts = Vec::with_capacity(hi - lo + 1);
            starts.push(0u64);
            for &d in &df[lo..hi] {
                starts.push(starts.last().copied().unwrap_or(0) + d);
            }
            let mut cursor = starts.clone();
            let mut postings = vec![(0u32, 0f32); total as usize];
            for d in 0..n_docs {
                for (c, v) in index.entries(d as u32) {
                    let c = c as usize;
                    if (lo..hi).contains(&c) {
                        let slot = &mut cursor[c - lo];
                        postings[*slot as usize] = (d as u32, v);
                        *slot += 1;
                    }
                }
            }
            let lists: Vec<&mut [(u32, f32)]> = {
                let mut rest = postings.as_mut_slice();
                let mut out = Vec::with_capacity(hi - lo);
                for t in 0..hi - lo {
                    let (head, tail) = rest.split_at_mut((starts[t + 1] - starts[t]) as usize);
                    out.push(head);
                    rest = tail;
                }
                out
            };
            let this = &index;
            let built: Vec<TermBlocks> = lists
                .into_par_iter()
                .enumerate()
                .map_init(
                    || BuildScratch::new(n_dims),
                    |scratch, (i, list)| this.cluster_list(lo + i, list, params, scratch),
                )
                .collect();
            for tb in built {
                index.append(tb);
            }
            lo = hi;
        }
        Ok(index)
    }

    /// Coordinates and stored values of document `d`.
    fn entries(&self, d: u32) -> impl Iterator<Item = (u16, f32)> + '_ {
        let (a, b) = (
            self.fwd_offsets[d as usize] as usize,
            self.fwd_offsets[d as usize + 1] as usize,
        );
        let row = &self.fwd[a..b];
        let n = (row.len().saturating_sub(4)) / 3;
        let scale = f32::from_le_bytes([row[0], row[1], row[2], row[3]]);
        (0..n).map(move |i| {
            let id = u16::from_le_bytes([row[4 + 2 * i], row[5 + 2 * i]]);
            (id, f32::from(row[4 + 2 * n + i]) * scale)
        })
    }

    /// Inner product of document `d` (after `build`) with a dense query.
    #[inline]
    fn score(&self, d: u32, dense: &[f32]) -> f32 {
        let (a, b) = (
            self.fwd_offsets[d as usize] as usize,
            self.fwd_offsets[d as usize + 1] as usize,
        );
        dot_row(&self.fwd[a..b], dense)
    }

    /// Hint the cache to load the offsets of document `d`.
    #[inline]
    fn prefetch_offsets(&self, d: u32) {
        prefetch(self.fwd_offsets[d as usize..].as_ptr().cast(), 16);
    }

    /// Hint the cache to load the forward row of document `d`.
    #[inline]
    fn prefetch_row(&self, d: u32) {
        let (a, b) = (
            self.fwd_offsets[d as usize] as usize,
            self.fwd_offsets[d as usize + 1] as usize,
        );
        prefetch(self.fwd[a..].as_ptr().cast(), b - a);
    }

    fn append(&mut self, tb: TermBlocks) {
        let mut doc_at = 0usize;
        for &dc in &tb.doc_counts {
            let dc = dc as usize;
            self.block_docs
                .extend_from_slice(&tb.docs[doc_at..doc_at + dc]);
            self.block_doc_offsets.push(self.block_docs.len() as u64);
            doc_at += dc;
        }
        self.n_blocks += tb.doc_counts.len();
        self.col_ids.extend_from_slice(&tb.col_ids);
        self.col_scales.extend_from_slice(&tb.col_scales);
        self.res_scales.push(tb.res_scale);
        self.col_codes.extend_from_slice(&tb.codes);
        self.term_cols.push(self.col_ids.len() as u64);
        self.term_code_off.push(self.col_codes.len() as u64);
        self.term_blocks.push(self.n_blocks as u32);
    }

    /// Prunes, clusters and summarizes the list of coordinate `term`.
    fn cluster_list(
        &self,
        term: usize,
        list: &mut [(u32, f32)],
        params: SparseBuildParams,
        scratch: &mut BuildScratch,
    ) -> TermBlocks {
        let mut out = TermBlocks {
            doc_counts: Vec::new(),
            docs: Vec::new(),
            col_ids: Vec::new(),
            col_scales: Vec::new(),
            res_scale: 0.0,
            codes: Vec::new(),
        };
        if list.is_empty() {
            return out;
        }
        let by_value_desc =
            |a: &(u32, f32), b: &(u32, f32)| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
        let keep = params.postings_per_term.min(list.len());
        if keep == 0 {
            return out;
        }
        if keep < list.len() {
            list.select_nth_unstable_by(keep - 1, by_value_desc);
        }
        let list = &mut list[..keep];

        let n_blocks = ((keep as f64 * f64::from(params.block_fraction)).ceil() as usize)
            .max(1)
            .min(keep);
        let assign: Vec<u32> = if n_blocks == 1 {
            vec![0; keep]
        } else if n_blocks == keep {
            (0..keep as u32).collect()
        } else {
            self.assign_to_centroids(term, list, n_blocks, params.seed, scratch)
        };

        // Group by block, documents ascending within a block.
        let mut order: Vec<u32> = (0..keep as u32).collect();
        order.sort_unstable_by_key(|&i| (assign[i as usize], list[i as usize].0));
        let mut groups: Vec<(f32, &[u32])> = Vec::with_capacity(n_blocks);
        let mut start = 0;
        while start < keep {
            let b = assign[order[start] as usize];
            let mut end = start;
            let mut top = 0f32;
            while end < keep && assign[order[end] as usize] == b {
                top = top.max(list[order[end] as usize].1);
                end += 1;
            }
            groups.push((top, &order[start..end]));
            start = end;
        }
        // Blocks with the largest value of this coordinate first: the threshold rises early.
        groups.sort_by(|a, b| b.0.total_cmp(&a.0));

        let mut maxima: Vec<Vec<(u16, f32)>> = Vec::with_capacity(groups.len());
        let mut members_of: Vec<Vec<u32>> = Vec::with_capacity(groups.len());
        for (_, members) in groups {
            let docs: Vec<u32> = members.iter().map(|&i| list[i as usize].0).collect();
            out.docs.extend_from_slice(&docs);
            out.doc_counts.push(docs.len() as u32);
            maxima.push(self.block_maxima(docs.iter().copied(), scratch));
            members_of.push(docs);
        }
        self.pack_columns(
            &maxima,
            &members_of,
            params.summary_coords,
            scratch,
            &mut out,
        );
        out
    }

    /// Picks `n_blocks` documents of `list` as centroids and assigns every document to the
    /// centroid with the largest inner product (ties to the lower centroid).
    fn assign_to_centroids(
        &self,
        term: usize,
        list: &[(u32, f32)],
        n_blocks: usize,
        seed: u64,
        scratch: &mut BuildScratch,
    ) -> Vec<u32> {
        let mut state = seed ^ (term as u64).wrapping_mul(0x2545_f491_4f6c_dd1d);
        // Partial Fisher-Yates over positions.
        let mut pos: Vec<u32> = (0..list.len() as u32).collect();
        for i in 0..n_blocks {
            let j = i + (splitmix(&mut state) % (list.len() - i) as u64) as usize;
            pos.swap(i, j);
        }
        // Inverted map of the centroids: coordinate -> (centroid, value).
        let mut entries: Vec<(u16, u32, f32)> = Vec::new();
        for (c, &p) in pos[..n_blocks].iter().enumerate() {
            entries.extend(
                self.entries(list[p as usize].0)
                    .map(|(t, v)| (t, c as u32, v)),
            );
        }
        entries.sort_unstable_by_key(|e| (e.0, e.1));
        let starts = &mut scratch.starts;
        let mut touched = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let t = e.0 as usize;
            if starts[t] == u32::MAX {
                starts[t] = i as u32;
                touched.push(t);
            }
        }
        let mut scores = vec![0f32; n_blocks];
        let mut assign = Vec::with_capacity(list.len());
        for &(doc, _) in list {
            scores.iter_mut().for_each(|s| *s = 0.0);
            for (t, v) in self.entries(doc) {
                let s = starts[t as usize];
                if s == u32::MAX {
                    continue;
                }
                for e in entries[s as usize..].iter().take_while(|e| e.0 == t) {
                    scores[e.1 as usize] += v * e.2;
                }
            }
            let mut best = 0usize;
            for (c, &s) in scores.iter().enumerate() {
                if s > scores[best] {
                    best = c;
                }
            }
            assign.push(best as u32);
        }
        for t in touched {
            starts[t] = u32::MAX;
        }
        assign
    }

    /// Coordinate-wise maxima of `docs`, ascending by coordinate.
    fn block_maxima(
        &self,
        docs: impl Iterator<Item = u32>,
        scratch: &mut BuildScratch,
    ) -> Vec<(u16, f32)> {
        let BuildScratch { dense, touched, .. } = scratch;
        for d in docs {
            for (t, v) in self.entries(d) {
                let slot = &mut dense[t as usize];
                if *slot == 0.0 {
                    touched.push(t);
                }
                *slot = slot.max(v);
            }
        }
        touched.sort_unstable();
        let coords = touched.iter().map(|&t| (t, dense[t as usize])).collect();
        for &t in touched.iter() {
            dense[t as usize] = 0.0;
        }
        touched.clear();
        coords
    }

    /// Writes the summary columns of a list: the `keep` coordinates with the largest summed
    /// block maxima, each quantized upward to one byte per block against its own maximum.
    fn pack_columns(
        &self,
        blocks: &[Vec<(u16, f32)>],
        members: &[Vec<u32>],
        keep: usize,
        scratch: &mut BuildScratch,
        out: &mut TermBlocks,
    ) {
        let BuildScratch {
            dense,
            touched,
            starts,
        } = scratch;
        for &(t, v) in blocks.iter().flatten() {
            let slot = &mut dense[t as usize];
            if *slot == 0.0 {
                touched.push(t);
            }
            *slot += v;
        }
        let mut coords: Vec<(u16, f32)> = touched.iter().map(|&t| (t, dense[t as usize])).collect();
        for &t in touched.iter() {
            dense[t as usize] = 0.0;
        }
        touched.clear();
        if keep == 0 {
            coords.clear();
        } else if coords.len() > keep {
            coords.select_nth_unstable_by(keep - 1, |a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            coords.truncate(keep);
        }
        coords.sort_unstable_by_key(|c| c.0);
        let stride = pad(blocks.len());
        for (j, &(t, _)) in coords.iter().enumerate() {
            starts[t as usize] = j as u32;
        }
        let mut maxima = vec![0f32; coords.len()];
        for &(t, v) in blocks.iter().flatten() {
            let j = starts[t as usize];
            if j != u32::MAX {
                let m = &mut maxima[j as usize];
                *m = m.max(v);
            }
        }
        // Above max / 255 so that the bound survives rounding in the summed inner products.
        let scales: Vec<f32> = maxima
            .iter()
            .map(|&m| {
                if m > 0.0 {
                    m / 255.0 * (1.0 + 1e-4)
                } else {
                    1.0
                }
            })
            .collect();
        let quantize = |v: f32, scale: f32| {
            let mut code = (v / scale).ceil().min(255.0) as u8;
            while code < 255 && f32::from(code) * scale < v {
                code += 1;
            }
            code
        };
        // Per block: the largest norm, over its documents, of the part outside the columns.
        let residual: Vec<f32> = members
            .iter()
            .map(|docs| {
                docs.iter()
                    .map(|&d| {
                        self.entries(d)
                            .filter(|&(t, _)| starts[t as usize] == u32::MAX)
                            .map(|(_, v)| v * v)
                            .sum::<f32>()
                    })
                    .fold(0f32, f32::max)
                    .sqrt()
            })
            .collect();
        let res_max = residual.iter().fold(0f32, |m, &v| m.max(v));
        let res_scale = if res_max > 0.0 {
            res_max / 255.0 * (1.0 + 1e-4)
        } else {
            1.0
        };
        out.res_scale = res_scale;
        out.codes.resize((coords.len() + 1) * stride, 0);
        for (b, blk) in blocks.iter().enumerate() {
            for &(t, v) in blk {
                let j = starts[t as usize];
                if j != u32::MAX {
                    out.codes[j as usize * stride + b] = quantize(v, scales[j as usize]);
                }
            }
            if residual[b] > 0.0 {
                out.codes[coords.len() * stride + b] = quantize(residual[b], res_scale);
            }
        }
        for &(t, _) in &coords {
            starts[t as usize] = u32::MAX;
        }
        out.col_ids.extend(coords.iter().map(|c| c.0));
        out.col_scales.extend(scales);
    }

    pub fn n_docs(&self) -> usize {
        self.n_docs
    }

    pub fn n_dims(&self) -> usize {
        self.n_dims
    }

    /// Bytes held by the index arrays.
    pub fn memory_bytes(&self) -> usize {
        self.fwd_offsets.len() * 8
            + self.fwd.len()
            + self.term_blocks.len() * 4
            + self.block_doc_offsets.len() * 8
            + self.block_docs.len() * 4
            + self.term_cols.len() * 8
            + self.col_ids.len() * 2
            + self.col_scales.len() * 4
            + self.res_scales.len() * 4
            + self.term_code_off.len() * 8
            + self.col_codes.len()
    }

    /// Number of blocks and kept postings.
    pub fn block_stats(&self) -> (usize, usize) {
        (self.n_blocks, self.block_docs.len())
    }

    /// Scratch for [`SparseSearcher::search`]; one per thread.
    pub fn searcher(&self) -> SparseSearcher<'_> {
        SparseSearcher {
            index: self,
            dense: vec![0.0; self.n_dims],
            dense_pos: vec![0.0; self.n_dims],
            visited: vec![0; self.n_docs.div_ceil(64)],
            touched: Vec::new(),
            heap: BinaryHeap::new(),
            pending: Vec::new(),
            ready: Vec::new(),
            bounds: Vec::new(),
            matched: Vec::new(),
            skipped_blocks: 0,
        }
    }

    /// The exact top-k by scanning the forward index (reference for tests and ground truth).
    pub fn brute_force(&self, ids: &[u32], vals: &[f32], k: usize) -> Vec<(u32, f32)> {
        let mut dense = vec![0f32; self.n_dims];
        for (&c, &v) in ids.iter().zip(vals) {
            if (c as usize) < self.n_dims {
                dense[c as usize] += v;
            }
        }
        let mut heap = BinaryHeap::new();
        for d in 0..self.n_docs as u32 {
            let s = self.score(d, &dense);
            if s > 0.0 {
                offer(&mut heap, k, Hit { score: s, doc: d });
            }
        }
        into_sorted(heap)
    }
}

/// Hint the cache to load `bytes` bytes at `ptr`. A no-op where no prefetch is wired up.
#[inline]
fn prefetch(ptr: *const u8, bytes: usize) {
    let end = ptr as usize + bytes;
    let mut line = ptr as usize & !63;
    while line < end {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: PRFM is a hint: it never faults and writes no register or memory, whatever
        // the address.
        unsafe {
            std::arch::asm!(
                "prfm pldl1keep, [{0}]",
                in(reg) line,
                options(nostack, preserves_flags, readonly)
            );
        }
        #[cfg(target_arch = "x86_64")]
        // SAFETY: PREFETCHT0 (SSE, part of the x86_64 baseline) is a hint: it never faults
        // and writes no register or memory, whatever the address.
        unsafe {
            std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                line as *const i8,
            );
        }
        line += 64;
    }
}

#[inline]
fn dot_row(row: &[u8], dense: &[f32]) -> f32 {
    let n = (row.len() - 4) / 3;
    #[cfg(target_arch = "x86_64")]
    if n >= 8 && simd::available() {
        // SAFETY: the features were detected; the row holds `n` ids and codes after its scale,
        // and every id is below `dense.len()` (they are validated build coordinates and
        // `dense` has one slot per coordinate).
        return unsafe { simd::dot_row(row, n, dense) };
    }
    let scale = f32::from_le_bytes([row[0], row[1], row[2], row[3]]);
    let mut sum = 0f32;
    for i in 0..n {
        let id = u16::from_le_bytes([row[4 + 2 * i], row[5 + 2 * i]]);
        sum += dense[id as usize] * f32::from(row[4 + 2 * n + i]);
    }
    sum * scale
}

/// Asks for transparent huge pages on the allocation of `v`: the forward index is read at
/// random, so 4 KiB pages make every row a TLB miss.
fn advise_huge_pages<T>(v: &Vec<T>) {
    #[cfg(target_os = "linux")]
    {
        const HUGE: usize = 2 << 20;
        let start = (v.as_ptr() as usize + HUGE - 1) & !(HUGE - 1);
        let end = (v.as_ptr() as usize + v.capacity() * std::mem::size_of::<T>()) & !(HUGE - 1);
        if end > start {
            // SAFETY: the range lies inside the live allocation of `v`; MADV_HUGEPAGE only
            // changes how the kernel backs it and never its contents.
            unsafe {
                libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_HUGEPAGE);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = v;
}

/// `bounds[b] += w * codes[j * stride + b]` for every `(j, w)` in `matched` and every block `b`
/// below `stride`, which is a multiple of 32.
#[inline]
fn accumulate_columns(bounds: &mut [f32], codes: &[u8], stride: usize, matched: &[(u32, f32)]) {
    debug_assert!(stride & 31 == 0 && bounds.len() >= stride);
    #[cfg(target_arch = "x86_64")]
    if simd::available() {
        // SAFETY: the features were detected; callers pass `bounds.len() >= stride` and every
        // `j` in `matched` has a column of `stride` bytes in `codes` (checked below).
        assert!(matched
            .iter()
            .all(|&(j, _)| (j as usize + 1) * stride <= codes.len()));
        unsafe { simd::accumulate_columns(bounds, codes, stride, matched) };
        return;
    }
    for &(j, w) in matched {
        let col = &codes[j as usize * stride..][..stride];
        for (acc, &c) in bounds[..stride].iter_mut().zip(col) {
            *acc += w * f32::from(c);
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod simd {
    use std::arch::x86_64::*;

    #[inline]
    pub fn available() -> bool {
        std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn hsum(v: __m256) -> f32 {
        let s = _mm_add_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps::<1>(v));
        let s = _mm_add_ps(s, _mm_movehl_ps(s, s));
        _mm_cvtss_f32(_mm_add_ss(s, _mm_shuffle_ps::<1>(s, s)))
    }

    /// # Safety
    /// AVX2 and FMA must be available, `row.len() == 4 + 3 * n` and every coordinate in the row
    /// must be below `dense.len()`.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_row(row: &[u8], n: usize, dense: &[f32]) -> f32 {
        debug_assert_eq!(row.len(), 4 + 3 * n);
        let scale = f32::from_le_bytes([row[0], row[1], row[2], row[3]]);
        let ids = row.as_ptr().add(4);
        let codes = row.as_ptr().add(4 + 2 * n);
        let term = |i: usize, acc: __m256| {
            let idx = _mm256_cvtepu16_epi32(_mm_loadu_si128(ids.add(2 * i).cast()));
            let c = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(_mm_loadl_epi64(codes.add(i).cast())));
            _mm256_fmadd_ps(_mm256_i32gather_ps::<4>(dense.as_ptr(), idx), c, acc)
        };
        let (mut a0, mut a1) = (_mm256_setzero_ps(), _mm256_setzero_ps());
        let mut i = 0;
        while i + 16 <= n {
            a0 = term(i, a0);
            a1 = term(i + 8, a1);
            i += 16;
        }
        if i + 8 <= n {
            a0 = term(i, a0);
            i += 8;
        }
        let mut sum = hsum(_mm256_add_ps(a0, a1));
        while i < n {
            let id = u16::from_le_bytes([row[4 + 2 * i], row[5 + 2 * i]]);
            sum += dense[id as usize] * f32::from(row[4 + 2 * n + i]);
            i += 1;
        }
        sum * scale
    }

    /// # Safety
    /// AVX2 and FMA must be available; `stride` is a multiple of 32, `bounds.len() >= stride`
    /// and every `(j, _)` satisfies `(j + 1) * stride <= codes.len()`.
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn accumulate_columns(
        bounds: &mut [f32],
        codes: &[u8],
        stride: usize,
        matched: &[(u32, f32)],
    ) {
        let mut b = 0;
        while b < stride {
            let mut acc = [_mm256_setzero_ps(); 4];
            for &(j, w) in matched {
                let wv = _mm256_set1_ps(w);
                let base = codes.as_ptr().add(j as usize * stride + b);
                for (l, a) in acc.iter_mut().enumerate() {
                    let c = _mm256_cvtepi32_ps(_mm256_cvtepu8_epi32(_mm_loadl_epi64(
                        base.add(l * 8).cast(),
                    )));
                    *a = _mm256_fmadd_ps(c, wv, *a);
                }
            }
            for (l, a) in acc.iter().enumerate() {
                let p = bounds.as_mut_ptr().add(b + l * 8);
                _mm256_storeu_ps(p, _mm256_add_ps(_mm256_loadu_ps(p), *a));
            }
            b += 32;
        }
    }
}

/// A scored document; `Ord` puts the worst result (lowest score, then highest id) on top of
/// the max-heap so it is the one evicted.
#[derive(Clone, Copy, PartialEq)]
struct Hit {
    score: f32,
    doc: u32,
}

impl Eq for Hit {}

impl Ord for Hit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.doc.cmp(&other.doc))
    }
}

impl PartialOrd for Hit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn offer(heap: &mut BinaryHeap<Hit>, k: usize, hit: Hit) {
    if heap.len() < k {
        heap.push(hit);
    } else if let Some(mut worst) = heap.peek_mut() {
        if hit < *worst {
            *worst = hit;
        }
    }
}

fn into_sorted(heap: BinaryHeap<Hit>) -> Vec<(u32, f32)> {
    heap.into_sorted_vec()
        .into_iter()
        .map(|h| (h.doc, h.score))
        .collect()
}

/// Per-thread search state.
pub struct SparseSearcher<'a> {
    index: &'a SparseImpactIndex,
    dense: Vec<f32>,
    dense_pos: Vec<f32>,
    visited: Vec<u64>,
    touched: Vec<u32>,
    heap: BinaryHeap<Hit>,
    pending: Vec<u32>,
    ready: Vec<u32>,
    bounds: Vec<f32>,
    matched: Vec<(u32, f32)>,
    /// Blocks passed over by the summary test, over the searcher's lifetime.
    skipped_blocks: u64,
}

/// Candidates gathered before their rows are fetched, and scored one batch later.
const SCORE_BATCH: usize = 16;

impl SparseSearcher<'_> {
    /// Fetches the rows of the pending batch while scoring the one fetched last time.
    fn rotate(&mut self, k: usize) {
        let index = self.index;
        for &d in &self.pending {
            index.prefetch_row(d);
        }
        for &d in &self.ready {
            let s = index.score(d, &self.dense);
            if s > 0.0 {
                offer(&mut self.heap, k, Hit { score: s, doc: d });
            }
        }
        self.ready.clear();
        std::mem::swap(&mut self.ready, &mut self.pending);
    }

    /// The top `k` documents by inner product with the query `(ids, vals)`, best first, as
    /// `(doc, score)`; documents scoring zero or less are never returned. Duplicate query
    /// coordinates add up; coordinates outside the index are ignored.
    pub fn search(
        &mut self,
        ids: &[u32],
        vals: &[f32],
        k: usize,
        params: SparseSearchParams,
    ) -> Vec<(u32, f32)> {
        let index = self.index;
        if k == 0 {
            return Vec::new();
        }
        let mut terms: Vec<(u32, f32)> = Vec::with_capacity(ids.len());
        for (&c, &v) in ids.iter().zip(vals) {
            if (c as usize) < index.n_dims && v.is_finite() && v != 0.0 {
                if self.dense[c as usize] == 0.0 && self.dense_pos[c as usize] == 0.0 {
                    terms.push((c, 0.0));
                }
                self.dense[c as usize] += v;
            }
        }
        for t in terms.iter_mut() {
            let w = self.dense[t.0 as usize];
            t.1 = w;
            self.dense_pos[t.0 as usize] = w.max(0.0);
        }
        terms.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let l2sq: f32 = terms.iter().map(|t| t.1.max(0.0) * t.1.max(0.0)).sum();
        let walked = terms
            .iter()
            .take(params.query_cut)
            .take_while(|t| t.1 > 0.0);

        self.heap.clear();
        for &(t, _) in walked {
            let blocks =
                index.term_blocks[t as usize] as usize..index.term_blocks[t as usize + 1] as usize;
            if blocks.is_empty() {
                continue;
            }
            let stride = pad(blocks.len());
            let (c0, c1) = (
                index.term_cols[t as usize] as usize,
                index.term_cols[t as usize + 1] as usize,
            );
            self.matched.clear();
            let mut kept_sq = 0f32;
            for (j, (&c, &scale)) in index.col_ids[c0..c1]
                .iter()
                .zip(&index.col_scales[c0..c1])
                .enumerate()
            {
                let w = self.dense_pos[c as usize];
                if w > 0.0 {
                    kept_sq += w * w;
                    self.matched.push((j as u32, w * scale));
                }
            }
            let res_w =
                params.residual * (l2sq - kept_sq).max(0.0).sqrt() * index.res_scales[t as usize];
            if res_w > 0.0 {
                self.matched.push(((c1 - c0) as u32, res_w));
            }
            self.bounds.clear();
            self.bounds.resize(stride, 0.0);
            let code_base = index.term_code_off[t as usize] as usize;
            let code_end = index.term_code_off[t as usize + 1] as usize;
            accumulate_columns(
                &mut self.bounds,
                &index.col_codes[code_base..code_end],
                stride,
                &self.matched,
            );
            let first = blocks.start;
            for b in blocks {
                if self.heap.len() == k {
                    let threshold = self.heap.peek().map_or(f32::MIN, |h| h.score);
                    if self.bounds[b - first] * params.heap_factor < threshold {
                        self.skipped_blocks += 1;
                        continue;
                    }
                }
                let (d0, d1) = (
                    index.block_doc_offsets[b] as usize,
                    index.block_doc_offsets[b + 1] as usize,
                );
                for &d in &index.block_docs[d0..d1] {
                    let (w, bit) = ((d >> 6) as usize, 1u64 << (d & 63));
                    if self.visited[w] & bit != 0 {
                        continue;
                    }
                    if self.visited[w] == 0 {
                        self.touched.push(w as u32);
                    }
                    self.visited[w] |= bit;
                    index.prefetch_offsets(d);
                    self.pending.push(d);
                    if self.pending.len() >= SCORE_BATCH {
                        self.rotate(k);
                    }
                }
            }
        }
        self.rotate(k);
        self.rotate(k);
        for &w in &self.touched {
            self.visited[w as usize] = 0;
        }
        self.touched.clear();
        for t in &terms {
            self.dense[t.0 as usize] = 0.0;
            self.dense_pos[t.0 as usize] = 0.0;
        }
        into_sorted(std::mem::take(&mut self.heap))
    }
}

impl SparseImpactIndex {
    /// Searches the CSR query rows in parallel; row `i` of the result holds query `i`'s ids,
    /// best first, padded with `u32::MAX`.
    pub fn search_batch(
        &self,
        queries: CsrView<'_>,
        k: usize,
        params: SparseSearchParams,
    ) -> Result<Vec<u32>, SparseIndexError> {
        let nq = queries.validate()?;
        let mut out = vec![u32::MAX; nq * k];
        if k == 0 {
            return Ok(out);
        }
        // A searcher holds dense query arrays and a visited bitset over all documents: give each
        // task several queries so the per-task setup is amortized.
        let min_len = nq.div_ceil(rayon::current_num_threads() * 4).max(1);
        out.par_chunks_mut(k)
            .enumerate()
            .with_min_len(min_len)
            .for_each_init(
                || (self.searcher(), Vec::new()),
                |(searcher, ids), (q, row)| {
                    let (qi, qv) = queries.row(q);
                    ids.clear();
                    ids.extend(qi.iter().map(|&c| c as u32));
                    for (slot, (doc, _)) in row.iter_mut().zip(searcher.search(ids, qv, k, params))
                    {
                        *slot = doc;
                    }
                },
            );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Csr {
        indptr: Vec<i64>,
        indices: Vec<i32>,
        data: Vec<f32>,
        n_dims: usize,
    }

    impl Csr {
        fn view(&self) -> CsrView<'_> {
            CsrView {
                indptr: &self.indptr,
                indices: &self.indices,
                data: &self.data,
                n_dims: self.n_dims,
            }
        }
    }

    /// Random rows with a skewed coordinate distribution; values on a coarse grid so that ties
    /// occur, some rows empty and some duplicated.
    fn random_csr(rows: usize, n_dims: usize, max_nnz: usize, state: &mut u64) -> Csr {
        let mut indptr = vec![0i64];
        let mut indices = Vec::new();
        let mut data = Vec::new();
        for r in 0..rows {
            if r % 17 == 5 && r > 0 {
                let (a, b) = (indptr[r - 1] as usize, indptr[r] as usize);
                indices.extend_from_within(a..b);
                data.extend_from_within(a..b);
            } else {
                let nnz = (splitmix(state) % (max_nnz as u64 + 1)) as usize;
                let mut cols: Vec<i32> = (0..nnz)
                    .map(|_| {
                        let u = splitmix(state) % (n_dims as u64 * n_dims as u64);
                        (u as f64).sqrt() as i32
                    })
                    .collect();
                cols.sort_unstable();
                cols.dedup();
                for c in cols {
                    indices.push(c);
                    data.push((1 + splitmix(state) % 8) as f32 * 0.25);
                }
            }
            indptr.push(indices.len() as i64);
        }
        Csr {
            indptr,
            indices,
            data,
            n_dims,
        }
    }

    #[test]
    fn exact_mode_matches_brute_force() {
        let mut state = 7u64;
        for (rows, n_dims, frac) in [(600, 64, 0.1f32), (2000, 300, 0.05), (50, 16, 0.5)] {
            let docs = random_csr(rows, n_dims, 24, &mut state);
            let params = SparseBuildParams {
                block_fraction: frac,
                transpose_budget: 997,
                ..SparseBuildParams::exact()
            };
            let index = SparseImpactIndex::build(docs.view(), params).unwrap();
            let queries = random_csr(80, n_dims + 4, 12, &mut state);
            let mut searcher = index.searcher();
            for q in 0..80 {
                let (a, b) = (queries.indptr[q] as usize, queries.indptr[q + 1] as usize);
                let ids: Vec<u32> = queries.indices[a..b].iter().map(|&c| c as u32).collect();
                let mut vals = queries.data[a..b].to_vec();
                if q % 9 == 0 && !vals.is_empty() {
                    vals[0] = -vals[0];
                }
                for k in [1, 10, 37] {
                    let want = index.brute_force(&ids, &vals, k);
                    let got = searcher.search(&ids, &vals, k, SparseSearchParams::exact());
                    assert_eq!(got, want, "rows {rows} query {q} k {k}");
                }
            }
            // The bound must actually prune, or this compares brute force with itself.
            assert!(
                searcher.skipped_blocks > 0,
                "rows {rows}: no block was skipped"
            );
        }
    }

    #[test]
    fn truncated_columns_with_full_residual_stay_exact() {
        let mut state = 11u64;
        for (rows, n_dims, frac, coords) in [
            (800, 64, 0.1f32, 6usize),
            (1500, 200, 0.25, 12),
            (300, 32, 1.0, 3),
        ] {
            let docs = random_csr(rows, n_dims, 24, &mut state);
            let params = SparseBuildParams {
                block_fraction: frac,
                summary_coords: coords,
                ..SparseBuildParams::exact()
            };
            let index = SparseImpactIndex::build(docs.view(), params).unwrap();
            let queries = random_csr(60, n_dims, 12, &mut state);
            let mut searcher = index.searcher();
            for q in 0..60 {
                let (a, b) = (queries.indptr[q] as usize, queries.indptr[q + 1] as usize);
                let ids: Vec<u32> = queries.indices[a..b].iter().map(|&c| c as u32).collect();
                let vals = &queries.data[a..b];
                let want = index.brute_force(&ids, vals, 10);
                let got = searcher.search(&ids, vals, 10, SparseSearchParams::exact());
                assert_eq!(got, want, "rows {rows} coords {coords} query {q}");
            }
        }
    }

    #[test]
    fn approximate_mode_returns_true_scores() {
        let mut state = 11u64;
        let docs = random_csr(3000, 200, 30, &mut state);
        let index = SparseImpactIndex::build(
            docs.view(),
            SparseBuildParams {
                postings_per_term: 100,
                ..SparseBuildParams::default()
            },
        )
        .unwrap();
        let params = SparseSearchParams {
            query_cut: 5,
            heap_factor: 0.8,
            residual: 0.5,
        };
        let mut searcher = index.searcher();
        let (ids, vals) = ([3u32, 40, 41, 150], [1.0f32, 0.5, 0.75, 2.0]);
        let got = searcher.search(&ids, &vals, 10, params);
        assert!(!got.is_empty());
        let mut dense = vec![0f32; 200];
        for (&c, &v) in ids.iter().zip(&vals) {
            dense[c as usize] = v;
        }
        for w in got.windows(2) {
            assert!(w[0].1 >= w[1].1);
        }
        for &(d, s) in &got {
            assert_eq!(s, index.score(d, &dense));
        }
        let batch = index
            .search_batch(
                CsrView {
                    indptr: &[0, 4],
                    indices: &[3, 40, 41, 150],
                    data: &vals,
                    n_dims: 200,
                },
                10,
                params,
            )
            .unwrap();
        let ids_only: Vec<u32> = got.iter().map(|h| h.0).collect();
        assert_eq!(&batch[..ids_only.len()], ids_only.as_slice());
    }

    #[test]
    fn stored_values_are_within_half_a_step() {
        let mut state = 5u64;
        let docs = random_csr(200, 40, 24, &mut state);
        let index = SparseImpactIndex::build(docs.view(), SparseBuildParams::exact()).unwrap();
        for d in 0..200u32 {
            let (_, want) = docs.view().row(d as usize);
            let max = want.iter().fold(0f32, |m, &v| m.max(v));
            let got: Vec<(u16, f32)> = index.entries(d).collect();
            let want_pos: Vec<f32> = want.iter().copied().filter(|&v| v > 0.0).collect();
            assert_eq!(got.len(), want_pos.len());
            for ((_, g), w) in got.iter().zip(&want_pos) {
                assert!(
                    *g > 0.0 && (g - w).abs() <= max / 510.0 * 1.001,
                    "{g} vs {w}"
                );
            }
        }
    }

    #[test]
    fn rejects_bad_input() {
        let bad = |indptr: Vec<i64>, indices: Vec<i32>, data: Vec<f32>| {
            SparseImpactIndex::build(
                CsrView {
                    indptr: &indptr,
                    indices: &indices,
                    data: &data,
                    n_dims: 4,
                },
                SparseBuildParams::exact(),
            )
            .is_err()
        };
        assert!(bad(vec![0, 1], vec![4], vec![1.0]));
        assert!(bad(vec![0, 1], vec![0], vec![-1.0]));
        assert!(bad(vec![0, 1], vec![0], vec![f32::NAN]));
        assert!(bad(vec![0, 2], vec![0], vec![1.0]));
        assert!(bad(vec![], vec![], vec![]));
    }
}
