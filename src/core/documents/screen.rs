// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Quantized screen of a store's embeddings for unfiltered hybrid queries.
//!
//! Every embedded chunk keeps two `i8` planes of its embedding: a coarse code with one scale per row and a second code of
//! the first's rounding residual at 1/254 of that scale. A query is quantized to 15 bits and a row's dot product with it
//! is one exact integer reduction per plane: the coarse plane screens every row, the fine plane the survivors. The quantization errors of the row and of the query
//! are measured when the codes are built and when the query arrives, so each row's exact cosine lies within a rigorous
//! interval of its screened value (`|q.v - q^.v^| <= |q||r| + |e||v^|` with `r`, `e` the row and query residuals,
//! Cauchy-Schwarz). The rows whose interval can still reach the list's `limit`-th best score or the floor are the
//! candidates; the caller scores only those with [`super::Embedding::cosine`], so every score it ranks is the exact
//! one and the screen changes which chunks are read, never a score.

use std::{cmp::Reverse, collections::BinaryHeap};

use super::Embedding;

/// Largest dimension the integer accumulators cannot overflow for: `127 * 2047 * 4096 < 2^31`.
pub(super) const MAX_DIMENSIONS: usize = 4096;
/// Largest query code; fewer levels are used where `127 * level * dimensions` would leave `i32`.
const MAX_QUERY_LEVEL: i64 = 32767;

#[derive(Clone, Copy, PartialEq)]
struct Row {
    /// Row code scale; `0` for an all-zero row.
    scale: f64,
    /// Upper bound of the l2 norm of the coarse plane's residual.
    residual: f64,
    /// Upper bound of the l2 norm of the residual left after both planes.
    fine_residual: f64,
    /// Upper bound of the l2 norm of the dequantized row.
    norm: f64,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// No embedding in this slot.
    None,
    Coded,
    /// An embedding the codes cannot represent (other length, non-finite value): always a candidate.
    Exact,
}

#[derive(Clone, PartialEq)]
pub(super) struct ScreenTable {
    dimensions: usize,
    codes: Vec<i8>,
    fine: Vec<i8>,
    rows: Vec<Row>,
}

impl ScreenTable {
    const NO_ROW: Row = Row {
        scale: 0.0,
        residual: 0.0,
        fine_residual: 0.0,
        norm: 0.0,
        kind: Kind::None,
    };

    /// The table of `slots` slots, all without an embedding.
    pub(super) fn empty(slots: usize, dimensions: usize) -> Self {
        Self {
            dimensions,
            codes: vec![0i8; slots * dimensions],
            fine: vec![0i8; slots * dimensions],
            rows: vec![Self::NO_ROW; slots],
        }
    }

    pub(super) fn build<'a>(
        slots: usize,
        dimensions: usize,
        embeddings: impl Iterator<Item = (usize, &'a Embedding)>,
    ) -> Self {
        let mut table = Self::empty(slots, dimensions);
        for (slot, embedding) in embeddings {
            table.set(slot, Some(embedding));
        }
        table
    }

    pub(super) fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Cuts the table to its first `slots` slots.
    pub(super) fn truncate(&mut self, slots: usize) {
        self.codes.truncate(slots * self.dimensions);
        self.fine.truncate(slots * self.dimensions);
        self.rows.truncate(slots);
    }

    /// Sets the row of `slot` (growing the table to hold it) to `embedding`, or to none.
    pub(super) fn set(&mut self, slot: usize, embedding: Option<&Embedding>) {
        let dimensions = self.dimensions;
        if slot >= self.rows.len() {
            self.codes.resize((slot + 1) * dimensions, 0);
            self.fine.resize((slot + 1) * dimensions, 0);
            self.rows.resize(slot + 1, Self::NO_ROW);
        }
        let span = slot * dimensions..(slot + 1) * dimensions;
        self.codes[span.clone()].fill(0);
        self.fine[span.clone()].fill(0);
        self.rows[slot] = Self::NO_ROW;
        let Some(embedding) = embedding else {
            return;
        };
        let values: &[f32] = embedding;
        let finite = values.iter().all(|value| value.is_finite());
        if values.len() != dimensions || !finite {
            self.rows[slot].kind = Kind::Exact;
            return;
        }
        let largest = values.iter().fold(0f64, |m, &v| m.max(f64::from(v).abs()));
        let scale = f64::from((largest / 127.0) as f32);
        let fine_scale = scale / 254.0;
        let (mut residual, mut fine_residual, mut norm) = (0f64, 0f64, 0f64);
        for ((code, fine_code), &value) in self.codes[span.clone()]
            .iter_mut()
            .zip(&mut self.fine[span])
            .zip(values)
        {
            let value = f64::from(value);
            let level = if scale > 0.0 {
                (value / scale).round().clamp(-127.0, 127.0)
            } else {
                0.0
            };
            *code = level as i8;
            let left = value - scale * level;
            let fine_level = if scale > 0.0 {
                (left / fine_scale).round().clamp(-127.0, 127.0)
            } else {
                0.0
            };
            *fine_code = fine_level as i8;
            let after = left - fine_scale * fine_level;
            residual += left * left;
            fine_residual += after * after;
            let decoded = scale * level + fine_scale * fine_level;
            norm += decoded * decoded;
        }
        self.rows[slot] = Row {
            scale,
            residual: residual.sqrt() * (1.0 + 1e-9) + 1e-12,
            fine_residual: fine_residual.sqrt() * (1.0 + 1e-9) + 1e-12,
            norm: norm.sqrt() * (1.0 + 1e-9) + 1e-12,
            kind: Kind::Coded,
        };
    }

    /// The slots that can be among the `limit` best exact scores at or above `floor`, ascending: every embedded slot
    /// whose interval reaches both. `None` when the query cannot be quantized (all zero or not finite).
    pub(super) fn candidates(&self, query: &[f64], limit: usize, floor: f64) -> Option<Vec<usize>> {
        if query.len() != self.dimensions || limit == 0 {
            return None;
        }
        let largest = query.iter().fold(0f64, |m, &q| m.max(q.abs()));
        if !(largest.is_finite() && largest > 0.0) {
            return None;
        }
        let levels =
            MAX_QUERY_LEVEL.min(i64::from(i32::MAX) / (127 * self.dimensions as i64)) as f64;
        let step = largest / levels;
        let mut norm = 0f64;
        let mut error = 0f64;
        let coded: Vec<i16> = query
            .iter()
            .map(|&q| {
                let level = (q / step).round().clamp(-levels, levels);
                norm += q * q;
                error += (q - step * level) * (q - step * level);
                level as i16
            })
            .collect();
        let norm = norm.sqrt();
        let error = error.sqrt() * (1.0 + 1e-9) + 1e-12;
        // The interval of a row's exact score from the screened `value` of the planes read, the l2 bound of the row
        // they leave unread and the l2 bound of the dequantized row they read.
        let span = |value: f64, residual: f64, dequantized: f64| {
            let slack = norm * residual + error * dequantized + 1e-9 + value.abs() * 1e-12;
            ((value - slack).max(-1.0), (value + slack).min(1.0))
        };
        let dot = dot_kernel();
        // One pass over the coarse plane. `best` holds the `limit` largest lower bounds so far; a row whose upper
        // bound is below the floor or below the smallest of them (which only rises) cannot rank, and the rest wait
        // for the fine plane.
        let mut best: BinaryHeap<Reverse<u64>> = BinaryHeap::with_capacity(limit + 1);
        let mut floor_key = order_key(floor);
        let mut waiting: Vec<(usize, f64, i32)> = Vec::new();
        let mut kept: Vec<usize> = Vec::new();
        for (slot, (row, codes)) in self
            .rows
            .iter()
            .zip(self.codes.chunks_exact(self.dimensions))
            .enumerate()
        {
            match row.kind {
                Kind::None => continue,
                Kind::Exact => {
                    kept.push(slot);
                    continue;
                }
                Kind::Coded => {}
            }
            let coarse = dot(codes, &coded);
            // The coarse plane alone differs from the full dequantization by at most both residuals.
            let dequantized = row.norm + row.residual + row.fine_residual;
            let (low, high) = span(
                f64::from(coarse) * row.scale * step,
                row.residual,
                dequantized,
            );
            if order_key(high) < floor_key {
                continue;
            }
            waiting.push((slot, high, coarse));
            let key = order_key(low);
            if best.len() < limit {
                best.push(Reverse(key));
            } else if key > best.peek().map_or(0, |Reverse(k)| *k) {
                best.pop();
                best.push(Reverse(key));
            } else {
                continue;
            }
            if best.len() == limit {
                floor_key = floor_key.max(best.peek().map_or(0, |Reverse(k)| *k));
            }
        }
        // The fine plane tightens each survivor's interval (the intersection of both); the cut is the `limit`-th
        // largest lower bound of the tightened intervals, or the floor.
        let mut refined: Vec<(usize, f64, f64)> = Vec::with_capacity(waiting.len());
        for (slot, high, coarse) in waiting {
            if order_key(high) < floor_key {
                continue;
            }
            let row = &self.rows[slot];
            let plane = &self.fine[slot * self.dimensions..(slot + 1) * self.dimensions];
            let value = (f64::from(coarse) * row.scale
                + f64::from(dot(plane, &coded)) * row.scale / 254.0)
                * step;
            let (low, tight) = span(value, row.fine_residual, row.norm);
            refined.push((slot, low, tight.min(high)));
        }
        let mut lower: Vec<f64> = refined.iter().map(|r| r.1).collect();
        let cut = if lower.len() > limit {
            let (_, nth, _) = lower.select_nth_unstable_by(limit - 1, |a, b| b.total_cmp(a));
            *nth
        } else {
            f64::NEG_INFINITY
        };
        let threshold = cut.max(floor);
        kept.extend(refined.iter().filter(|r| r.2 >= threshold).map(|r| r.0));
        kept.sort_unstable();
        Some(kept)
    }
}

/// A `u64` that orders as the `f64` does for every finite value.
fn order_key(value: f64) -> u64 {
    let bits = value.to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

type DotKernel = fn(&[i8], &[i16]) -> i32;

fn dot_kernel() -> DotKernel {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        return |row, query| {
            // SAFETY: the CPU supports avx2, checked above.
            unsafe { dot_avx2(row, query) }
        };
    }
    dot_portable
}

fn dot_portable(row: &[i8], query: &[i16]) -> i32 {
    let mut out = 0;
    reduce_one(row, query, &mut out);
    out
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_avx2(row: &[i8], query: &[i16]) -> i32 {
    use std::arch::x86_64::*;
    let n = row.len().min(query.len());
    let mut acc = _mm256_setzero_si256();
    let mut i = 0;
    while i + 16 <= n {
        let r = _mm256_cvtepi8_epi16(_mm_loadu_si128(row.as_ptr().add(i).cast()));
        let q = _mm256_loadu_si256(query.as_ptr().add(i).cast());
        acc = _mm256_add_epi32(acc, _mm256_madd_epi16(r, q));
        i += 16;
    }
    let sum = _mm_add_epi32(
        _mm256_castsi256_si128(acc),
        _mm256_extracti128_si256(acc, 1),
    );
    let sum = _mm_hadd_epi32(sum, sum);
    let sum = _mm_hadd_epi32(sum, sum);
    let mut total = _mm_cvtsi128_si32(sum);
    while i < n {
        total += i32::from(row[i]) * i32::from(query[i]);
        i += 1;
    }
    total
}

#[inline(always)]
fn reduce_one(row: &[i8], query: &[i16], out: &mut i32) {
    let mut lanes = [0i32; 16];
    let (rows, row_tail) = row.as_chunks::<16>();
    let (queries, query_tail) = query.as_chunks::<16>();
    for (r, q) in rows.iter().zip(queries) {
        for j in 0..16 {
            lanes[j] += i32::from(r[j]) * i32::from(q[j]);
        }
    }
    let tail: i32 = row_tail
        .iter()
        .zip(query_tail)
        .map(|(&r, &q)| i32::from(r) * i32::from(q))
        .sum();
    *out = lanes.iter().sum::<i32>() + tail;
}
