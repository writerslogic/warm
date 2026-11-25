// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Lists to probe for one query under an adaptive rule. `d2` is the query's squared distance to
/// every centre and `radii` the per-list radii (read only by `Radius`). Candidates are the `nmax`
/// nearest lists; the `nmin` nearest are always kept.
pub(super) fn select_probes(
    d2: &[f32],
    radii: &[f32],
    rule: ProbeRule,
    nmin: usize,
    nmax: usize,
) -> Vec<u32> {
    let nmax = nmax.clamp(1, d2.len());
    let nmin = nmin.clamp(1, nmax);
    let mut idx: Vec<u32> = (0..d2.len() as u32).collect();
    idx.select_nth_unstable_by(nmax - 1, |a, b| d2[*a as usize].total_cmp(&d2[*b as usize]));
    idx.truncate(nmax);
    idx.sort_unstable_by(|a, b| d2[*a as usize].total_cmp(&d2[*b as usize]));
    let d1sq = d2[idx[0] as usize];
    match rule {
        ProbeRule::Fixed => {}
        ProbeRule::Ratio { eps } => {
            let cut = d1sq * (1.0 + eps);
            let keep = idx.partition_point(|&i| d2[i as usize] <= cut);
            idx.truncate(keep.max(nmin));
        }
        ProbeRule::Radius { eps, scale } => {
            let cut = (1.0 + eps) * d1sq.sqrt();
            let mut pos = 0;
            idx.retain(|&i| {
                pos += 1;
                pos <= nmin || (d2[i as usize].sqrt() - scale * radii[i as usize]).max(0.0) <= cut
            });
        }
    }
    idx
}

/// Per-(query, list) table build cost in scanned-code equivalents.
pub(super) const TABLE_CODES: u64 = 1300;

/// Assigns queries to `threads` workers by estimated scan work (one table plus the codes of each
/// probed list), longest first onto the least-loaded worker. Returns each query's worker, its
/// index within that worker, and each worker's queries in that index order.
pub(super) fn balance_queries(
    probe: &[Vec<u32>],
    lists: &[List],
    threads: usize,
) -> (Vec<u32>, Vec<u32>, Vec<Vec<u32>>) {
    let cost = |q: usize| -> u64 {
        probe[q]
            .iter()
            .map(|&li| lists[li as usize].slots.len() as u64 + TABLE_CODES)
            .sum()
    };
    let mut order: Vec<(u64, u32)> = (0..probe.len()).map(|q| (cost(q), q as u32)).collect();
    order.sort_unstable_by(|a, b| b.cmp(a));
    let mut load = vec![0u64; threads];
    let (mut owner, mut local) = (vec![0u32; probe.len()], vec![0u32; probe.len()]);
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); threads];
    for (c, q) in order {
        let th = (0..threads).min_by_key(|&i| (load[i], i)).unwrap_or(0);
        load[th] += c;
        owner[q as usize] = th as u32;
        local[q as usize] = members[th].len() as u32;
        members[th].push(q);
    }
    (owner, local, members)
}

/// Packs a score and a slot into one `u64` whose integer order is the score's total order, then
/// the slot's: the scan's candidate buffers select and sort on plain integers.
#[inline(always)]
pub(super) fn score_key(score: f32, slot: u32) -> u64 {
    let b = score.to_bits();
    let ord = if b >> 31 == 1 { !b } else { b | 0x8000_0000 };
    (u64::from(ord) << 32) | u64::from(slot)
}

/// The score of a [`score_key`].
#[inline(always)]
pub(super) fn key_score(key: u64) -> f32 {
    let o = (key >> 32) as u32;
    f32::from_bits(if o >> 31 == 1 { o & 0x7fff_ffff } else { !o })
}

/// The u32 sums a scan admits under the running threshold `thr`: a code with sum `v` is kept when
/// `v < ((thr - base) / scale).max(0)`, which for integer `v` is `v < ceil(..)`.
#[inline]
pub(super) fn admit_cut(thr: f32, base: f32, scale: f32) -> u32 {
    if thr.is_finite() {
        ((thr - base) / scale).max(0.0).ceil() as u32
    } else {
        u32::MAX
    }
}

/// Scores one quad-layout block against `live` of the [`QB`] tables, table `i` being
/// `luts[sel[i] * lstride..]`; returns the admission masks by `i`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn scan_group(
    live: usize,
    codes: &[u8],
    luts: &[u8],
    lstride: usize,
    m: usize,
    sel: &[usize; QB],
    cuts: &[u32; QB],
    outs: &mut [[u32; BLOCK]; QB],
) -> [u32; QB] {
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn run<const K: usize>(
        codes: &[u8],
        luts: &[u8],
        lstride: usize,
        m: usize,
        sel: &[usize; QB],
        cuts: &[u32; QB],
        outs: &mut [[u32; BLOCK]; QB],
    ) -> [u32; QB] {
        let mut res = [0u32; QB];
        let masks = scan32q_lt::<K>(
            codes,
            luts,
            lstride,
            m,
            sel[..K].try_into().expect("K <= QB"),
            cuts[..K].try_into().expect("K <= QB"),
            (&mut outs[..K]).try_into().expect("K <= QB"),
        );
        res[..K].copy_from_slice(&masks);
        res
    }
    match live {
        1 => run::<1>(codes, luts, lstride, m, sel, cuts, outs),
        2 => run::<2>(codes, luts, lstride, m, sel, cuts, outs),
        3 => run::<3>(codes, luts, lstride, m, sel, cuts, outs),
        4 => run::<4>(codes, luts, lstride, m, sel, cuts, outs),
        5 => run::<5>(codes, luts, lstride, m, sel, cuts, outs),
        6 => run::<6>(codes, luts, lstride, m, sel, cuts, outs),
        7 => run::<7>(codes, luts, lstride, m, sel, cuts, outs),
        _ => run::<8>(codes, luts, lstride, m, sel, cuts, outs),
    }
}

/// `c[m x n] = a[m x k] . b[n x k]^T`, single-threaded.
pub(super) fn gemm_nt(a: &[f32], m: usize, b: &[f32], n: usize, k: usize, c: &mut [f32]) {
    assert!(a.len() >= m * k && b.len() >= n * k && c.len() >= m * n);
    if m == 0 || n == 0 {
        return;
    }
    // SAFETY: the asserts bound every access: A is m x k row-major, B^T is read from b as k x n
    // with row stride 1 and column stride k, C is m x n row-major and overwritten.
    unsafe {
        gemm::gemm(
            m,
            n,
            k,
            c.as_mut_ptr(),
            1,
            n as isize,
            false,
            a.as_ptr(),
            1,
            k as isize,
            b.as_ptr(),
            k as isize,
            1,
            0.0f32,
            1.0f32,
            false,
            false,
            false,
            gemm::Parallelism::None,
        );
    }
}

/// The `k` smallest `(value, index)` of `vals` (unordered), by a running worst-of-k filter.
#[cfg(test)]
pub(super) fn smallest_k(vals: &[f32], k: usize, out: &mut Vec<(f32, u32)>) {
    smallest_k_by(vals.len(), k, |i| vals[i], out);
}

pub(super) fn smallest_k_by(
    n: usize,
    k: usize,
    val: impl Fn(usize) -> f32,
    out: &mut Vec<(f32, u32)>,
) {
    out.clear();
    let k = k.min(n);
    if k == 0 {
        return;
    }
    out.extend((0..k).map(|i| (val(i), i as u32)));
    let (mut wi, mut wv) = worst_of(out);
    for i in k..n {
        let v = val(i);
        if v < wv {
            out[wi] = (v, i as u32);
            (wi, wv) = worst_of(out);
        }
    }
}

pub(super) fn worst_of(o: &[(f32, u32)]) -> (usize, f32) {
    o.iter()
        .enumerate()
        .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
        .map(|(i, x)| (i, x.0))
        .unwrap()
}

/// The `k` smallest `(c2[i] - 2 row[i], i)`, identical to [`smallest_k`] over those values (same
/// members in the same slots); eight values at a time are compared with the running worst and
/// only the chunks that hold a candidate take the scalar path.
pub(super) fn smallest_k_centres(row: &[f32], c2: &[f32], k: usize, out: &mut Vec<(f32, u32)>) {
    debug_assert_eq!(row.len(), c2.len());
    #[cfg(target_arch = "x86_64")]
    if k > 0 && k <= row.len() && std::is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 detected; `row` and `c2` have equal length.
        unsafe { smallest_k_centres_avx2(row, c2, k, out) };
        return;
    }
    smallest_k_by(row.len(), k, |i| c2[i] - 2.0 * row[i], out);
}

/// Index of the last maximum of the 32 lanes in `v` (padding lanes are `-inf`) and its value, as
/// [`worst_of`] picks them; `None` when the maximum is zero or not finite, where `total_cmp`
/// and float equality disagree.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn worst32(v: &[f32; 32]) -> Option<(usize, f32)> {
    use std::arch::x86_64::*;
    let p = v.as_ptr();
    let (a, b, c, d) = (
        _mm256_loadu_ps(p),
        _mm256_loadu_ps(p.add(8)),
        _mm256_loadu_ps(p.add(16)),
        _mm256_loadu_ps(p.add(24)),
    );
    let m = _mm256_max_ps(_mm256_max_ps(a, b), _mm256_max_ps(c, d));
    let m = _mm256_max_ps(m, _mm256_permute2f128_ps::<1>(m, m));
    let m = _mm256_max_ps(m, _mm256_shuffle_ps::<0b01_00_11_10>(m, m));
    let m = _mm256_max_ps(m, _mm256_shuffle_ps::<0b10_11_00_01>(m, m));
    let mx = _mm256_cvtss_f32(m);
    if mx == 0.0 || !mx.is_finite() {
        return None;
    }
    let mm = |x| _mm256_movemask_ps(_mm256_cmp_ps::<_CMP_EQ_OQ>(x, m)) as u32;
    let mask = mm(a) | (mm(b) << 8) | (mm(c) << 16) | (mm(d) << 24);
    Some((31 - mask.leading_zeros() as usize, mx))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub(super) unsafe fn smallest_k_centres_avx2(
    row: &[f32],
    c2: &[f32],
    k: usize,
    out: &mut Vec<(f32, u32)>,
) {
    use std::arch::x86_64::*;
    if k > 32 {
        smallest_k_by(row.len(), k, |i| c2[i] - 2.0 * row[i], out);
        return;
    }
    let val = |i: usize| c2[i] - 2.0 * row[i];
    out.clear();
    out.extend((0..k).map(|i| (val(i), i as u32)));
    let mut vals = [f32::NEG_INFINITY; 32];
    for (slot, o) in vals.iter_mut().zip(out.iter()) {
        *slot = o.0;
    }
    let worst =
        |out: &[(f32, u32)], vals: &[f32; 32]| worst32(vals).unwrap_or_else(|| worst_of(out));
    let (mut wi, mut wv) = worst(out, &vals);
    let n = row.len();
    let two = _mm256_set1_ps(2.0);
    let mut i = k;
    while i < n {
        if i + 8 <= n {
            let v = _mm256_sub_ps(
                _mm256_loadu_ps(c2.as_ptr().add(i)),
                _mm256_mul_ps(two, _mm256_loadu_ps(row.as_ptr().add(i))),
            );
            let lt = _mm256_cmp_ps::<_CMP_LT_OQ>(v, _mm256_set1_ps(wv));
            if _mm256_movemask_ps(lt) == 0 {
                i += 8;
                continue;
            }
        }
        for j in i..(i + 8).min(n) {
            let v = val(j);
            if v < wv {
                out[wi] = (v, j as u32);
                vals[wi] = v;
                (wi, wv) = worst(out, &vals);
            }
        }
        i += 8;
    }
}
