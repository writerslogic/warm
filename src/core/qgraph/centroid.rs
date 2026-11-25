// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Per-cluster f64 sums for a Lloyd step, parallel and bit-identical to the serial row-order sum.

use rayon::prelude::*;

/// Dimensions summed per task when there are too few clusters to occupy every worker.
const DIM_CHUNK: usize = 32;

/// `(sums, counts)` of `rows` (row-major, `dim` wide) by `labels` over `k` clusters.
///
/// Rows are bucketed by label with a stable counting sort, then each cluster adds its rows in
/// ascending row order. Every output element is therefore the same left-to-right f64 sum the
/// serial loop produces, whatever the thread count or task split.
pub(crate) fn centroid_sums(
    rows: &[f32],
    dim: usize,
    labels: &[u32],
    k: usize,
) -> (Vec<f64>, Vec<usize>) {
    let mut counts = vec![0usize; k];
    for &a in labels {
        counts[a as usize] += 1;
    }
    let mut start = vec![0usize; k + 1];
    for j in 0..k {
        start[j + 1] = start[j] + counts[j];
    }
    let mut next = start.clone();
    let mut order = vec![0u32; labels.len()];
    for (i, &a) in labels.iter().enumerate() {
        order[next[a as usize]] = i as u32;
        next[a as usize] += 1;
    }
    let mut sums = vec![0f64; k * dim];
    let split_dims = k < 4 * rayon::current_num_threads();
    sums.par_chunks_mut(dim).enumerate().for_each(|(j, s)| {
        let members = &order[start[j]..start[j + 1]];
        let add = |t0: usize, s: &mut [f64]| {
            for &i in members {
                let row = &rows[i as usize * dim + t0..][..s.len()];
                for (v, &x) in s.iter_mut().zip(row) {
                    *v += f64::from(x);
                }
            }
        };
        if split_dims && members.len() * dim >= 1 << 16 {
            s.par_chunks_mut(DIM_CHUNK)
                .enumerate()
                .for_each(|(c, s)| add(c * DIM_CHUNK, s));
        } else {
            add(0, s);
        }
    });
    (sums, counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_equal_the_serial_row_order_sum_for_any_thread_count() {
        let (n, dim, k) = (3000, 70, 9);
        let mut s = 0x1234_5678_u64;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u32
        };
        let rows: Vec<f32> = (0..n * dim).map(|_| rnd() as f32 / 1e6 - 1000.0).collect();
        let labels: Vec<u32> = (0..n).map(|_| rnd() % k as u32).collect();
        let mut want = vec![0f64; k * dim];
        let mut want_n = vec![0usize; k];
        for (i, &a) in labels.iter().enumerate() {
            want_n[a as usize] += 1;
            for t in 0..dim {
                want[a as usize * dim + t] += f64::from(rows[i * dim + t]);
            }
        }
        for threads in [1, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let (got, got_n) = pool.install(|| centroid_sums(&rows, dim, &labels, k));
            assert_eq!(got_n, want_n);
            assert!(got
                .iter()
                .zip(&want)
                .all(|(a, b)| a.to_bits() == b.to_bits()));
        }
    }
}
