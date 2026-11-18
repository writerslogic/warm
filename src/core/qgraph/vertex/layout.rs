// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Breadth-first order of the bottom layer from the entry (unreached vertices follow in id
/// order): neighbours get nearby positions, so a search touches fewer distinct pages and lines.
pub(in super::super) fn bfs_order(g: GraphView<'_>) -> Vec<u32> {
    let mut seen = vec![false; g.n];
    let mut order = Vec::with_capacity(g.n);
    for s in std::iter::once(g.entry as usize).chain(0..g.n) {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        let mut head = order.len();
        order.push(s as u32);
        while head < order.len() {
            let v = order[head] as usize;
            head += 1;
            for &u in g.neighbors(v) {
                if !std::mem::replace(&mut seen[u as usize], true) {
                    order.push(u);
                }
            }
        }
    }
    order
}

pub(super) struct VertexLayout {
    pub(super) padded: usize,
    pub(super) cbytes: usize,
    pub(super) stride: usize,
    pub(super) res_stride: usize,
}

impl VertexLayout {
    pub(super) fn new(
        n: usize,
        dim: usize,
        bits: u8,
        residual_bits: u8,
        align: bool,
        id_bytes: u8,
    ) -> Result<Self, WarmError> {
        if !matches!(bits, 4 | 8) {
            return Err(invalid(format!(
                "vertex code bits must be 4 or 8, not {bits}"
            )));
        }
        if !(id_bytes == 4 || id_bytes == 0 || (id_bytes == 3 && (n as u64) < u64::from(NONE3))) {
            return Err(invalid(format!("id_bytes must be 4, 0 (delta varints), or 3 below 2^24 - 1 vertices (got {id_bytes} for {n} vertices)")));
        }
        if !matches!(residual_bits, 0 | 4 | 8) {
            return Err(invalid(format!(
                "residual code bits must be 0, 4 or 8, not {residual_bits}"
            )));
        }
        let padded = dim.next_multiple_of(if bits == 4 { 32 } else { 16 });
        let cbytes = if bits == 4 { padded / 2 } else { padded };
        let stride = match (align, cbytes) {
            (false, _) => cbytes,
            (true, c) if c <= 64 => c.next_power_of_two(),
            (true, c) => c.next_multiple_of(128),
        };
        let res_stride = match residual_bits {
            8 => padded,
            4 => padded / 2,
            _ => 0,
        };
        Ok(Self {
            padded,
            cbytes,
            stride,
            res_stride,
        })
    }
}
