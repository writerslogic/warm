// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Experimental partition tier for streaming L2 search (the L22 design): an IVF of `lists`
//! k-means centres trained once, every row in its nearest list plus one SOAR spill list
//! (Sun et al., 2023), residual PQ codes with 16 centroids per sub-space stored in the
//! 32-code LUT16 blocks of the `fastscan` kernel, an INT8 table per (query, list), and an exact
//! reorder of the best candidates from the resident f32 rows.
//!
//! Before training the tier answers by exact brute force. Training happens at the first search
//! with at least `train_at` live rows; centres are then fixed (no list maintenance).
//!
//! Insert assignment uses one rows x centres product per block: the `spill_k` nearest centres
//! by L2 give the exact nearest list, and the spill list is the SOAR minimiser among the other
//! `spill_k - 1`, computed from `x.c` and the centre Gram matrix (no second product).

use std::collections::HashMap;

use rayon::prelude::*;

use super::exact_l2;
use super::fastscan::{
    build_lut, quad_available, quad_pos, quad_stride, scan32_lt, scan32q_lt, BLOCK,
};
use super::invalid;
use crate::core::error::WarmError;

/// How many lists a query probes, decided at routing time from its sorted centre distances.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ProbeRule {
    /// The `nprobe` nearest lists.
    #[default]
    Fixed,
    /// List `i` while `d_i^2 <= d_1^2 (1 + eps)`.
    Ratio { eps: f32 },
    /// List `i` while `max(0, d_i - scale * r_i) <= (1 + eps) d_1`, with `r_i` the list's member
    /// radius (mean + 2 sd of member-to-centre distance). Not monotone in `i`: a far list with a
    /// large radius is kept past a near list with a small one.
    Radius { eps: f32, scale: f32 },
}

/// Tier parameters. Defaults are the stage-1 operating point.
#[derive(Clone, Debug)]
pub struct PartitionParams {
    pub lists: usize,
    /// Train at the first search with at least this many live rows.
    pub train_at: usize,
    pub train_sample: usize,
    pub kmeans_iters: usize,
    /// PQ sub-spaces (must divide the dimension), 16 centroids each.
    pub pq_m: usize,
    pub pq_sample: usize,
    pub pq_iters: usize,
    /// Candidate centres for the spill choice (nearest `spill_k` by L2).
    pub spill_k: usize,
    pub soar_lambda: f32,
    /// Lists probed by `ProbeRule::Fixed`; the cap for the adaptive rules.
    pub nprobe: usize,
    /// Floor on the lists an adaptive rule probes.
    pub nprobe_min: usize,
    pub probe_rule: ProbeRule,
    pub reorder: usize,
    pub seed: u64,
    /// Ascending live-row counts. After the first training, the first search that finds at
    /// least this many live rows retrains the centres and codebook and re-places every row;
    /// each count fires once. Counts at or below the live size of the previous training are
    /// skipped.
    pub retrain_at: Vec<usize>,
    /// Retrain at a search when the list imbalance (`lists * sum(len^2) / sum(len)^2`) exceeds
    /// this (0 disables) and at least `retrain_min_inserts` rows were inserted since the last
    /// training.
    pub retrain_imb: f32,
    pub retrain_min_inserts: usize,
    /// Lloyd iterations of a retrain warm-started from the current centres on a sample of
    /// `retrain_sample` live rows (0 = a cold k-means as at the first training).
    pub retrain_warm_iters: usize,
    pub retrain_sample: usize,
    /// Reseed near-empty centres onto the largest clusters between Lloyd iterations (in the
    /// first training and in retrains), which restores balance after the data has moved.
    pub kmeans_rebalance: bool,
    /// `(live rows, factor)` points, ascending in rows, scaling `nprobe` by the live size at
    /// each search (piecewise linear, flat outside the points). Empty = a constant `nprobe`.
    /// Lists shrink with the live set, so a small set can afford more probes per query.
    pub nprobe_by_live: Vec<(usize, f32)>,
    /// A row also joins its SOAR spill list only when that list's centre is within
    /// `(1 + spill_eps)` times the squared distance to the primary centre; infinity spills
    /// every row.
    pub spill_eps: f32,
}

impl Default for PartitionParams {
    fn default() -> Self {
        Self {
            lists: 5000,
            train_at: 1_000_000,
            train_sample: 262_144,
            kmeans_iters: 10,
            pq_m: 50,
            pq_sample: 32_000,
            pq_iters: 10,
            spill_k: 32,
            soar_lambda: 1.0,
            nprobe: 350,
            nprobe_min: 1,
            probe_rule: ProbeRule::Fixed,
            reorder: 317,
            seed: 7,
            retrain_at: Vec::new(),
            retrain_imb: 0.0,
            retrain_min_inserts: 1_000_000,
            retrain_warm_iters: 0,
            retrain_sample: 262_144,
            kmeans_rebalance: false,
            nprobe_by_live: Vec::new(),
            spill_eps: f32::INFINITY,
        }
    }
}

/// Rows per assignment / brute-force block (the block's product is `ROWS x lists` floats).
const ROWS: usize = 1024;
const BF_QUERIES: usize = 256;
const BF_ROWS: usize = 8192;
/// Rows placed per step when a whole tier is (re)placed.
const PLACE_CHUNK: usize = 1 << 18;

#[derive(Default)]
struct List {
    /// `blocks * 16 * m` bytes in the LUT16 block layout.
    codes: Vec<u8>,
    /// Slot of each code, in position order.
    slots: Vec<u32>,
    /// Sum and sum of squares of member-to-centre distance over `slots` (spilled members included).
    dist_sum: f64,
    dist_sq: f64,
}

struct Trained {
    centres: Vec<f32>,
    c2: Vec<f32>,
    /// Centre Gram matrix `C C^T`, row-major `lists x lists`.
    gram: Vec<f32>,
    /// `m x 16 x sub` codebook.
    codebook: Vec<f32>,
    /// Per-list table term `|v|^2 + 2 <c_b, v>`, `lists x 16m`.
    list_tab: Vec<f32>,
    lists: Vec<List>,
    /// Per slot: (list, position) of the primary and spill memberships; `u32::MAX` = none.
    loc: Vec<[(u32, u32); 2]>,
}

/// Wall seconds of the phases of the last partition search (max over threads for the
/// per-thread phases).
#[derive(Clone, Copy, Debug, Default)]
pub struct SearchProfile {
    pub centres_s: f64,
    pub route_s: f64,
    /// Slowest thread's scan phase (table builds included).
    pub scan_s: f64,
    /// Mean over threads of the scan phase, to expose imbalance against `scan_s`.
    pub scan_mean_s: f64,
    /// Mean over threads of the time spent building per-(query, list) tables.
    pub lut_mean_s: f64,
    /// Lists probed, summed over queries.
    pub probes: u64,
    /// 32-code blocks scanned, summed over threads.
    pub blocks: u64,
    /// Candidates admitted below the running threshold, summed over threads.
    pub admitted: u64,
    pub reorder_s: f64,
    pub total_s: f64,
}

/// One search thread's (query, ids) results with its scan, reorder and
/// table-build seconds and its scanned block count.
type ThreadPart = (Vec<(u32, Vec<u32>)>, f64, f64, f64, u64, u64);

/// Streaming partition tier over external `u32` ids.
pub struct PartitionTier {
    dim: usize,
    params: PartitionParams,
    rows: Vec<f32>,
    slot_id: Vec<u32>,
    id_slot: HashMap<u32, u32, foldhash::fast::RandomState>,
    free: Vec<u32>,
    trained: Option<Trained>,
    shape: Shape,
    profile: std::sync::Mutex<SearchProfile>,
    /// Live rows at the last training, and the next unfired `retrain_at` entry.
    trained_live: usize,
    retrain_next: usize,
    inserts_since_train: usize,
    /// Seconds of the last training: centres, codebook and tables, placement.
    train_phases: [f64; 3],
}

use exact_l2::sq;

/// Membership of slots seen so far in one reorder: open addressing over a power-of-two table
/// sized for at most `cap` members, cleared between queries.
struct SlotSet {
    table: Vec<u32>,
    shift: u32,
}

impl SlotSet {
    const EMPTY: u32 = u32::MAX;

    fn new(cap: usize) -> Self {
        let size = (2 * cap).next_power_of_two().max(16);
        Self {
            table: vec![Self::EMPTY; size],
            shift: 32 - size.trailing_zeros(),
        }
    }

    fn clear(&mut self) {
        self.table.fill(Self::EMPTY);
    }

    /// True when `slot` was not yet a member.
    fn insert(&mut self, slot: u32) -> bool {
        let mask = self.table.len() - 1;
        let mut i = (slot.wrapping_mul(0x9E37_79B1) >> self.shift) as usize;
        loop {
            match self.table[i] {
                Self::EMPTY => {
                    self.table[i] = slot;
                    return true;
                }
                v if v == slot => return false,
                _ => i = (i + 1) & mask,
            }
        }
    }
}

mod kmeans;
mod scan;
mod tables;
#[cfg(test)]
mod tests;
mod tier;
use self::kmeans::*;
use self::scan::*;
use self::tables::*;
