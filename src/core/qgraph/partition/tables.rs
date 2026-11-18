// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Queries scored per pass over a block by the quad kernel.
pub(super) const QB: usize = 8;

/// How a list stores its codes: `m` nibbles per code in blocks of [`BLOCK`] codes, in either the
/// LUT16 shuffle layout (`16 * m` bytes per block) or, on AVX-512 VBMI/VNNI hardware, the quad
/// layout (see [`quad_pos`]).
#[derive(Clone, Copy)]
pub(super) struct Shape {
    pub(super) m: usize,
    pub(super) quad: bool,
}

impl Shape {
    pub(super) fn new(m: usize) -> Self {
        Self {
            m,
            quad: quad_available(),
        }
    }

    /// Bytes per block.
    pub(super) fn stride(self) -> usize {
        if self.quad {
            quad_stride(self.m)
        } else {
            16 * self.m
        }
    }

    pub(super) fn pos(self, i: usize, j: usize) -> usize {
        if self.quad {
            quad_pos(i, j)
        } else {
            16 * j + (i & 15)
        }
    }

    /// Nibble `j` of code position `i` within `block`.
    #[inline]
    pub(super) fn get(self, block: &[u8], i: usize, j: usize) -> u8 {
        let byte = block[self.pos(i, j)];
        if i < 16 {
            byte & 15
        } else {
            byte >> 4
        }
    }

    #[inline]
    pub(super) fn set(self, block: &mut [u8], i: usize, j: usize, v: u8) {
        let byte = &mut block[self.pos(i, j)];
        if i < 16 {
            *byte = (*byte & 0xF0) | v;
        } else {
            *byte = (*byte & 0x0F) | (v << 4);
        }
    }
}

impl List {
    /// Member radius: mean + 2 sd of member-to-centre distance (about its 97.7th percentile).
    pub(super) fn radius(&self) -> f32 {
        let n = self.slots.len();
        if n == 0 {
            return 0.0;
        }
        let mean = self.dist_sum / n as f64;
        let var = (self.dist_sq / n as f64 - mean * mean).max(0.0);
        (mean + 2.0 * var.sqrt()) as f32
    }

    /// Adds (`sign` = 1) or removes (`sign` = -1) one member's distance to the radius statistics.
    pub(super) fn track(&mut self, dist: f32, sign: f64) {
        let d = f64::from(dist);
        self.dist_sum += sign * d;
        self.dist_sq += sign * d * d;
    }

    pub(super) fn append(&mut self, sh: Shape, code: &[u8], slot: u32) -> u32 {
        let (pos, stride) = (self.slots.len(), sh.stride());
        if pos.is_multiple_of(BLOCK) {
            self.codes.resize(self.codes.len() + stride, 0);
        }
        let block = &mut self.codes[(pos / BLOCK) * stride..(pos / BLOCK + 1) * stride];
        for (j, &v) in code.iter().enumerate() {
            sh.set(block, pos % BLOCK, j, v);
        }
        self.slots.push(slot);
        pos as u32
    }

    /// Swap-removes position `pos`; returns the slot moved into `pos`, if any.
    pub(super) fn remove(&mut self, sh: Shape, pos: usize) -> Option<u32> {
        let (last, stride) = (self.slots.len() - 1, sh.stride());
        let moved = if pos != last {
            let (lb, li) = (last / BLOCK, last % BLOCK);
            let code: Vec<u8> = (0..sh.m)
                .map(|j| sh.get(&self.codes[lb * stride..(lb + 1) * stride], li, j))
                .collect();
            let block = &mut self.codes[(pos / BLOCK) * stride..(pos / BLOCK + 1) * stride];
            for (j, &v) in code.iter().enumerate() {
                sh.set(block, pos % BLOCK, j, v);
            }
            self.slots[pos] = self.slots[last];
            Some(self.slots[pos])
        } else {
            None
        };
        self.slots.pop();
        if self.slots.len().is_multiple_of(BLOCK) {
            self.codes.truncate((self.slots.len() / BLOCK) * stride);
        } else {
            // Clear the vacated code so padding lanes stay zero.
            let (lb, li) = (last / BLOCK, last % BLOCK);
            let block = &mut self.codes[lb * stride..(lb + 1) * stride];
            for j in 0..sh.m {
                sh.set(block, li, j, 0);
            }
        }
        moved
    }
}

impl Trained {
    pub(super) fn lists_len(&self) -> usize {
        self.c2.len()
    }

    /// Nearest list and SOAR spill list for each row of `x` (`b` rows).
    pub(super) fn assign(
        &self,
        x: &[f32],
        dim: usize,
        k: usize,
        lambda: f32,
        eps: f32,
    ) -> Vec<(u32, u32)> {
        let l = self.lists_len();
        let mut out = vec![(0u32, 0u32); x.len() / dim];
        out.par_chunks_mut(ROWS)
            .zip(x.par_chunks(ROWS * dim))
            .for_each(|(o, xb)| {
                let b = o.len();
                let mut xc = vec![0f32; b * l];
                gemm_nt(xb, b, &self.centres, l, dim, &mut xc);
                let mut cand = Vec::with_capacity(k);
                for (i, oi) in o.iter_mut().enumerate() {
                    let row = &xc[i * l..(i + 1) * l];
                    smallest_k_centres(row, &self.c2, k, &mut cand);
                    let c1 = cand
                        .iter()
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|x| x.1)
                        .unwrap() as usize;
                    let xi = &xb[i * dim..(i + 1) * dim];
                    let ci = &self.centres[c1 * dim..(c1 + 1) * dim];
                    let rr: f64 = xi
                        .iter()
                        .zip(ci)
                        .map(|(a, c)| f64::from(a - c).powi(2))
                        .sum::<f64>()
                        .max(1e-20);
                    let xr: f64 = xi
                        .iter()
                        .zip(ci)
                        .map(|(a, c)| f64::from(*a) * f64::from(a - c))
                        .sum();
                    let x2: f64 = xi.iter().map(|a| f64::from(*a).powi(2)).sum();
                    let g = &self.gram[c1 * l..(c1 + 1) * l];
                    let mut best = (f64::INFINITY, c1 as u32);
                    let mut best_dist = 0f64;
                    for &(_, c) in &cand {
                        let c = c as usize;
                        if c == c1 {
                            continue;
                        }
                        let xcc = f64::from(row[c]);
                        let dist = x2 - 2.0 * xcc + f64::from(self.c2[c]);
                        // <x - c, r> with r = x - c1: <x, r> - (<c, x> - <c, c1>).
                        let proj = xr - (xcc - f64::from(g[c]));
                        let loss = dist + f64::from(lambda) * proj * proj / rr;
                        if loss < best.0 {
                            best = (loss, c as u32);
                            best_dist = dist;
                        }
                    }
                    let spill = best.1 != c1 as u32 && best_dist <= (1.0 + f64::from(eps)) * rr;
                    *oi = (c1 as u32, if spill { best.1 } else { u32::MAX });
                }
            });
        out
    }

    /// PQ nibbles of the residual `x - c_list`.
    pub(super) fn encode(&self, x: &[f32], list: usize, dim: usize, m: usize, code: &mut [u8]) {
        let sub = dim / m;
        let c = &self.centres[list * dim..(list + 1) * dim];
        for b in 0..m {
            let mut best = (f32::INFINITY, 0u8);
            for v in 0..16 {
                let cb = &self.codebook[(b * 16 + v) * sub..(b * 16 + v + 1) * sub];
                let mut s = 0f32;
                for t in 0..sub {
                    let r = x[b * sub + t] - c[b * sub + t] - cb[t];
                    s += r * r;
                }
                if s < best.0 {
                    best = (s, v as u8);
                }
            }
            code[b] = best.1;
        }
    }
}
