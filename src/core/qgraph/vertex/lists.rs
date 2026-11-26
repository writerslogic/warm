// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Query lists: candidate lists learned from construction queries.
//!
//! The construction queries are clustered (Lloyd k-means in the coordinates the searcher
//! receives queries in); list `c` holds the union of the nearest base rows of the queries
//! assigned to centre `c`. A search with `probes > 0` estimates every row of the lists of the
//! `probes` centres nearest the query before (or instead of) the graph traversal, so a query
//! drawn from the construction queries' distribution starts with most of its neighbours in the
//! pool. The lists only ever come from construction queries, never from evaluation data.

use rayon::prelude::*;

use super::super::centroid::centroid_sums;
use super::super::invalid;
use super::super::kernels::{dot, SplitMix};
use crate::core::error::WarmError;

/// Lloyd iterations of the list centres.
const ITERS: usize = 12;
/// First bytes of the lists' file block.
const MAGIC: &[u8; 8] = b"HMSQLST1";
/// Seed of the centres' initial sample (fixed: lists are a deterministic function of the input).
const SEED: u64 = 0x9E37_1157_0000_0001;

/// Candidate lists of a [`super::VGraph`] (see the module documentation).
pub struct QueryLists {
    dim: usize,
    /// Unit-normalize a query before probing (angular indexes cluster unit queries).
    unit: bool,
    centres: Vec<f32>,
    /// `|c|^2` per centre.
    c2: Vec<f32>,
    /// List `c` is `ids[offsets[c]..offsets[c + 1]]`, sorted ascending.
    offsets: Vec<u64>,
    /// Vertex ids: base row ids until the lists are attached to an index, then its internal ids.
    ids: Vec<u32>,
    /// The listed rows' codes and scales, copied in list order (see [`QueryLists::pack`]).
    packed: Option<Packed>,
}

/// Row codes and scales of every listed id, in `ids` order, so that a list is scanned as one
/// contiguous run instead of a gather over the index's rows.
struct Packed {
    cbytes: usize,
    codes: Vec<i8>,
    scales: Vec<f32>,
}

/// Upper bound on the bytes a packed copy may take; larger lists are scanned by gather.
const PACK_MAX_BYTES: usize = 1 << 31;

impl QueryLists {
    /// Lists from `queries` (row-major, `dim` wide, in the coordinates the index is queried in)
    /// and their nearest base rows `neighbours` (`per_query` row ids per query, nearest first,
    /// each below `n`): `lists` centres, each list the union of the first `take` neighbours of
    /// its queries. `unit` normalizes queries (and later the probing query) to unit length.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        queries: &[f32],
        dim: usize,
        unit: bool,
        neighbours: &[u32],
        per_query: usize,
        take: usize,
        lists: usize,
        n: usize,
    ) -> Result<Self, WarmError> {
        if dim == 0 || !queries.len().is_multiple_of(dim) || queries.is_empty() {
            return Err(invalid("query lists need a non-empty query matrix".into()));
        }
        let count = queries.len() / dim;
        if per_query == 0 || neighbours.len() != count * per_query {
            return Err(invalid(format!(
                "query lists need {per_query} neighbours for each of {count} queries"
            )));
        }
        if lists == 0 || take == 0 {
            return Err(invalid(
                "query lists need positive list and neighbour counts".into(),
            ));
        }
        if let Some(&bad) = neighbours.iter().find(|&&v| v as usize >= n) {
            return Err(invalid(format!(
                "query list neighbour {bad} is not below {n}"
            )));
        }
        let mut rows = queries.to_vec();
        if unit {
            for row in rows.chunks_exact_mut(dim) {
                normalize(row);
            }
        }
        let lists = lists.min(count);
        let (centres, owner) = kmeans(&rows, dim, lists, ITERS, SEED);
        let c2: Vec<f32> = centres.chunks_exact(dim).map(|c| dot(c, c)).collect();
        let take = take.min(per_query);
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); lists];
        for (q, &c) in owner.iter().enumerate() {
            members[c as usize].extend_from_slice(&neighbours[q * per_query..q * per_query + take]);
        }
        let mut offsets = Vec::with_capacity(lists + 1);
        offsets.push(0u64);
        let mut ids = Vec::new();
        for mut list in members {
            list.sort_unstable();
            list.dedup();
            ids.extend_from_slice(&list);
            offsets.push(ids.len() as u64);
        }
        Ok(Self {
            dim,
            unit,
            centres,
            c2,
            offsets,
            ids,
            packed: None,
        })
    }

    /// Number of lists.
    pub fn lists(&self) -> usize {
        self.c2.len()
    }

    /// Total ids over all lists (a row listed under several centres counts once per list).
    pub fn listed(&self) -> usize {
        self.ids.len()
    }

    /// Bytes held by the lists.
    pub fn bytes(&self) -> usize {
        4 * (self.centres.len() + self.c2.len() + self.ids.len()) + 8 * self.offsets.len()
    }

    /// Replaces every id with `map[id]` (base row ids to an index's internal ids); each list is
    /// re-sorted so its scan walks memory forwards.
    pub(super) fn remap(&mut self, map: &[u32]) -> Result<(), WarmError> {
        if let Some(&bad) = self.ids.iter().find(|&&v| v as usize >= map.len()) {
            return Err(invalid(format!("query list id {bad} is outside the index")));
        }
        self.packed = None;
        for v in &mut self.ids {
            *v = map[*v as usize];
        }
        for c in 0..self.lists() {
            let (a, b) = (self.offsets[c] as usize, self.offsets[c + 1] as usize);
            self.ids[a..b].sort_unstable();
        }
        Ok(())
    }

    /// Dimension of the centres (and of the queries a search probes with).
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The trailing file block (see `file.rs`): magic, then `dim`, `unit`, list count and id
    /// count as `u64`, the centres (`f32`), the offsets (`u64`) and the ids (`u32`).
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(40 + self.bytes());
        out.extend_from_slice(MAGIC);
        for x in [
            self.dim,
            usize::from(self.unit),
            self.lists(),
            self.ids.len(),
        ] {
            out.extend_from_slice(&(x as u64).to_le_bytes());
        }
        self.centres
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        self.offsets
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        self.ids
            .iter()
            .for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        out
    }

    /// Reads [`QueryLists::encode`]'s block for an index of `n` vertices queried in `dim`
    /// coordinates; every count, offset and id is checked.
    pub(super) fn decode(bytes: &[u8], dim: usize, n: usize) -> Result<Self, WarmError> {
        let bad = |what: &str| invalid(format!("query lists block: {what}"));
        if bytes.len() < 40 || &bytes[..8] != MAGIC {
            return Err(bad("bad magic"));
        }
        let word = |i: usize| u64::from_le_bytes(bytes[8 + 8 * i..16 + 8 * i].try_into().unwrap());
        let (d, unit, lists, count) = (word(0), word(1), word(2), word(3));
        if d != dim as u64 || unit > 1 || lists == 0 || lists > u64::from(u32::MAX) {
            return Err(bad("header does not match the index"));
        }
        let (lists, count) = (lists as usize, usize::try_from(count).unwrap_or(usize::MAX));
        let centres_at = 40usize;
        let sizes = lists
            .checked_mul(dim)
            .and_then(|c| c.checked_mul(4))
            .and_then(|c| c.checked_add(centres_at))
            .and_then(|o| Some((o, o.checked_add(lists.checked_add(1)?.checked_mul(8)?)?)))
            .and_then(|(o, i)| Some((o, i, i.checked_add(count.checked_mul(4)?)?)));
        let Some((offsets_at, ids_at, end)) = sizes else {
            return Err(bad("sizes overflow"));
        };
        if bytes.len() != end {
            return Err(bad("length does not match the header"));
        }
        let centres: Vec<f32> = bytes[centres_at..offsets_at]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        let offsets: Vec<u64> = bytes[offsets_at..ids_at]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_le_bytes(*c))
            .collect();
        let ids: Vec<u32> = bytes[ids_at..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        if offsets[0] != 0
            || offsets[lists] != count as u64
            || offsets.windows(2).any(|w| w[0] > w[1])
        {
            return Err(bad("offsets are not monotone over the ids"));
        }
        if ids.iter().any(|&v| v as usize >= n) || centres.iter().any(|v| !v.is_finite()) {
            return Err(bad("an id or centre is out of range"));
        }
        let c2 = centres.chunks_exact(dim).map(|c| dot(c, c)).collect();
        Ok(Self {
            dim,
            unit: unit == 1,
            centres,
            c2,
            offsets,
            ids,
            packed: None,
        })
    }

    /// Copies the code and scale of every listed id (`row(id)` of the index the ids belong to,
    /// `cbytes` code bytes each) into list order. Skipped, leaving gather scans, when the copy
    /// would exceed [`PACK_MAX_BYTES`] or cannot be allocated.
    pub(super) fn pack<'a>(&mut self, cbytes: usize, row: impl Fn(u32) -> (&'a [i8], f32)) {
        self.packed = None;
        let rows = self.ids.len();
        if cbytes == 0
            || rows
                .checked_mul(cbytes + 4)
                .is_none_or(|b| b > PACK_MAX_BYTES)
        {
            return;
        }
        let mut codes: Vec<i8> = Vec::new();
        let mut scales: Vec<f32> = Vec::new();
        if codes.try_reserve_exact(rows * cbytes).is_err()
            || scales.try_reserve_exact(rows).is_err()
        {
            return;
        }
        for &v in &self.ids {
            let (code, scale) = row(v);
            debug_assert_eq!(code.len(), cbytes);
            codes.extend_from_slice(code);
            scales.push(scale);
        }
        self.packed = Some(Packed {
            cbytes,
            codes,
            scales,
        });
    }

    /// List `c` as `(ids, codes, scales)`, when the lists are packed.
    pub(super) fn packed_list(&self, c: u32) -> Option<(&[u32], &[i8], &[f32])> {
        let p = self.packed.as_ref()?;
        let (a, b) = (
            self.offsets[c as usize] as usize,
            self.offsets[c as usize + 1] as usize,
        );
        Some((
            &self.ids[a..b],
            &p.codes[a * p.cbytes..b * p.cbytes],
            &p.scales[a..b],
        ))
    }

    /// Drops the packed copy, so scans gather rows from the index.
    #[cfg(test)]
    pub(super) fn unpack(&mut self) {
        self.packed = None;
    }

    /// True when [`QueryLists::packed_list`] answers.
    pub(super) fn is_packed(&self) -> bool {
        self.packed.is_some()
    }

    /// The ids of list `c`.
    pub(super) fn list(&self, c: u32) -> &[u32] {
        let c = c as usize;
        &self.ids[self.offsets[c] as usize..self.offsets[c + 1] as usize]
    }

    /// The `probes` lists whose centres are nearest `query`, nearest first, into `out`
    /// (`scratch` holds the query copy when it is normalized).
    pub(super) fn probe(
        &self,
        query: &[f32],
        probes: usize,
        scratch: &mut Vec<f32>,
        out: &mut Vec<(f32, u32)>,
    ) {
        out.clear();
        let probes = probes.min(self.lists());
        if probes == 0 || query.len() != self.dim {
            return;
        }
        let q = if self.unit {
            scratch.clear();
            scratch.extend_from_slice(query);
            normalize(scratch);
            &scratch[..]
        } else {
            query
        };
        // `|q - c|^2 - |q|^2`: the query's own norm is common to every centre.
        for (c, (centre, &cc)) in self
            .centres
            .chunks_exact(self.dim)
            .zip(&self.c2)
            .enumerate()
        {
            let d = cc - 2.0 * dot(q, centre);
            if out.len() < probes {
                out.push((d, c as u32));
                let mut i = out.len() - 1;
                while i > 0 && out[i - 1].0 > out[i].0 {
                    out.swap(i - 1, i);
                    i -= 1;
                }
            } else if d < out[probes - 1].0 {
                out[probes - 1] = (d, c as u32);
                let mut i = probes - 1;
                while i > 0 && out[i - 1].0 > out[i].0 {
                    out.swap(i - 1, i);
                    i -= 1;
                }
            }
        }
    }
}

/// Nearest centre of every row (`c2[c] = |c|^2`), in parallel.
fn assign(rows: &[f32], dim: usize, centres: &[f32], c2: &[f32]) -> Vec<u32> {
    rows.par_chunks_exact(dim)
        .map(|row| {
            let mut best = (f32::INFINITY, 0u32);
            for (c, (centre, &cc)) in centres.chunks_exact(dim).zip(c2).enumerate() {
                let d = cc - 2.0 * dot(row, centre);
                if d < best.0 {
                    best = (d, c as u32);
                }
            }
            best.1
        })
        .collect()
}

/// Lloyd k-means over `rows`: `k` centres initialized from distinct sampled rows, `iters`
/// iterations, an empty cluster reseeded from a random row. Returns the centres and the final
/// assignment of every row.
fn kmeans(rows: &[f32], dim: usize, k: usize, iters: usize, seed: u64) -> (Vec<f32>, Vec<u32>) {
    let n = rows.len() / dim;
    let mut rng = SplitMix(seed);
    let mut order: Vec<usize> = (0..n).collect();
    for i in 0..k {
        let j = i + (rng.next_u64() % (n - i) as u64) as usize;
        order.swap(i, j);
    }
    let mut centres = Vec::with_capacity(k * dim);
    for &i in &order[..k] {
        centres.extend_from_slice(&rows[i * dim..(i + 1) * dim]);
    }
    let mut owner = Vec::new();
    for it in 0..=iters {
        let c2: Vec<f32> = centres.chunks_exact(dim).map(|c| dot(c, c)).collect();
        owner = assign(rows, dim, &centres, &c2);
        if it == iters {
            break;
        }
        let (sums, counts) = centroid_sums(rows, dim, &owner, k);
        for c in 0..k {
            let centre = &mut centres[c * dim..(c + 1) * dim];
            if counts[c] == 0 {
                let r = (rng.next_u64() % n as u64) as usize;
                centre.copy_from_slice(&rows[r * dim..(r + 1) * dim]);
            } else {
                for (v, s) in centre.iter_mut().zip(&sums[c * dim..(c + 1) * dim]) {
                    *v = (s / counts[c] as f64) as f32;
                }
            }
        }
    }
    (centres, owner)
}

fn normalize(row: &mut [f32]) {
    let norm = dot(row, row).sqrt();
    if norm > 0.0 {
        row.iter_mut().for_each(|v| *v /= norm);
    }
}
