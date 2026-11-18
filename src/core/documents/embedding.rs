// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// A chunk's dense embedding as stored: the l2-normalised values rounded to `f32`, in one
/// shared allocation that the store and every vector snapshot covering the chunk hold
/// together, so a snapshot adds no copy of the matrix (4 bytes per value in memory, against
/// the 8 of the `f64` form plus a 4-byte snapshot copy before this type).
///
/// Scoring: every path (the generic search, the dense index, the vector snapshot's exact
/// re-score) calls [`Embedding::cosine`], which widens each stored value to `f64` and
/// accumulates in `f64`, so one chunk has one semantic score however it was reached. The
/// score is the exact cosine of the stored (`f32`) vector; it differs from the cosine of the
/// client's `f64` vector by at most the rounding of the stored values, `2^-24` relative per
/// coordinate (about `6e-8` worst case over an l2-normalised pair, `2e-9` typical at 384
/// dimensions).
///
/// Serialized form: a JSON array of the `f32` values (shortest round-trip decimals, about
/// half the bytes of the `f64` form). A record written before this type held an array of
/// `f64` values under the same key; both read as an array of numbers rounded to the nearest
/// `f32`, so a store from either version opens, and a store written now still opens with
/// the previous release (which reads the array as `f64`). Chunks without an embedding
/// serialize byte for byte as before.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Embedding(Arc<[f32]>);

impl Embedding {
    /// Rounds l2-normalised `f64` values to the stored `f32` form.
    pub(crate) fn from_f64(values: &[f64]) -> Self {
        Self(values.iter().map(|&value| value as f32).collect())
    }

    pub(crate) fn from_f32(values: Vec<f32>) -> Self {
        Self(values.into())
    }

    /// The stored values widened to `f64` (exact).
    pub(crate) fn to_f64(&self) -> Vec<f64> {
        self.0.iter().map(|&value| f64::from(value)).collect()
    }

    /// The semantic score of this chunk against an l2-normalised `f64` query: the sequential
    /// `f64` dot product over the widened stored values, clamped to the cosine range.
    #[inline]
    pub(crate) fn cosine(&self, query: &[f64]) -> f64 {
        query
            .iter()
            .zip(self.0.iter())
            .map(|(q, &v)| q * f64::from(v))
            .sum::<f64>()
            .clamp(-1.0, 1.0)
    }

    /// `cosine` of four embeddings at once: four independent sequential reductions advanced together, so each score
    /// equals `self.cosine(query)` bit for bit. Falls back to one at a time unless all four match the query's length.
    #[inline]
    pub(crate) fn cosine4(rows: [&Self; 4], query: &[f64]) -> [f64; 4] {
        if rows.iter().any(|row| row.0.len() != query.len()) {
            return rows.map(|row| row.cosine(query));
        }
        let (mut a, mut b, mut c, mut d) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (i, &q) in query.iter().enumerate() {
            a += q * f64::from(rows[0].0[i]);
            b += q * f64::from(rows[1].0[i]);
            c += q * f64::from(rows[2].0[i]);
            d += q * f64::from(rows[3].0[i]);
        }
        [a, b, c, d].map(|sum| sum.clamp(-1.0, 1.0))
    }

    /// Screening scores for `rows` against an `f32` query: per row the `f32` dot product and the squared `f32` norm of
    /// the stored values, `(NaN, NaN)` for a row whose length differs from the query's. Both are inexact; see
    /// [`ScreenBound`] for the error they carry relative to [`Embedding::cosine`].
    pub(crate) fn screen(rows: &[&Self], query: &[f32], out: &mut Vec<(f32, f32)>) {
        out.clear();
        out.reserve(rows.len());
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: the CPU supports avx2, checked above.
            unsafe { screen_avx2(rows, query, out) };
            return;
        }
        screen_rows(rows, query, out);
    }

    /// Whether both handles share one allocation (a snapshot row and the store's chunk).
    #[cfg(test)]
    pub(crate) fn shares_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Rigorous bound on `|screen dot - Embedding::cosine|` before the cosine's clamp (which only shrinks it).
///
/// With `n` terms, `u = 2^-24` and `q` the `f64` query: the `f32` accumulation errs by at most `gamma_n * sum|q32 v|`
/// (any summation order, fused or not), rounding the query to `f32` adds `u * sum|q v|`, the sequential `f64`
/// reference errs by `n * 2^-53 * sum|q v|`, and `sum|q v| <= |q| |v|` by Cauchy-Schwarz. `|v|` is the square root of
/// the screened `f32` squared norm, itself within `(1 + gamma_n)`; the factor below covers it with room.
pub(crate) struct ScreenBound {
    pub(super) scale: f64,
}

impl ScreenBound {
    pub(crate) fn new(query: &[f64]) -> Self {
        let n = query.len() as f64;
        let u = 2f64.powi(-24);
        let norm = query.iter().map(|q| q * q).sum::<f64>().sqrt();
        Self {
            scale: 1.05 * (n + 8.0) * u * (1.0 + 4.0 * n * u) * norm,
        }
    }

    /// The bound for a row with the screened squared norm `norm_sq`; infinite when it is not finite.
    pub(crate) fn of(&self, norm_sq: f32) -> f64 {
        if norm_sq.is_finite() {
            self.scale * f64::from(norm_sq).sqrt()
        } else {
            f64::INFINITY
        }
    }
}

#[inline(always)]
pub(super) fn screen_rows(rows: &[&Embedding], query: &[f32], out: &mut Vec<(f32, f32)>) {
    for row in rows {
        out.push(screen_row(&row.0, query));
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn screen_avx2(rows: &[&Embedding], query: &[f32], out: &mut Vec<(f32, f32)>) {
    screen_rows(rows, query, out);
}

/// Sixteen independent `f32` lanes for the dot product and eight for the norm, so the loop vectorises on any target.
#[inline(always)]
pub(super) fn screen_row(row: &[f32], query: &[f32]) -> (f32, f32) {
    if row.len() != query.len() {
        return (f32::NAN, f32::NAN);
    }
    let mut dot = [0f32; 16];
    let mut norm = [0f32; 8];
    let (rows, row_tail) = row.as_chunks::<16>();
    let (queries, query_tail) = query.as_chunks::<16>();
    for (r, q) in rows.iter().zip(queries) {
        for j in 0..16 {
            dot[j] += r[j] * q[j];
        }
        for j in 0..8 {
            norm[j] += r[j] * r[j] + r[j + 8] * r[j + 8];
        }
    }
    let (mut tail_dot, mut tail_norm) = (0f32, 0f32);
    for (r, q) in row_tail.iter().zip(query_tail) {
        tail_dot += r * q;
        tail_norm += r * r;
    }
    let dot: f32 = dot.iter().sum::<f32>() + tail_dot;
    let norm: f32 = norm.iter().sum::<f32>() + tail_norm;
    (dot, norm)
}

impl Deref for Embedding {
    type Target = [f32];

    fn deref(&self) -> &[f32] {
        &self.0
    }
}

impl Serialize for Embedding {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter())
    }
}

impl<'de> Deserialize<'de> for Embedding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<f32>::deserialize(deserializer).map(Self::from_f32)
    }
}
