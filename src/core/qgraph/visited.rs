// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#[cfg(any(feature = "experimental", feature = "experimental-build-search"))]
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Candidate {
    pub(super) dist: f32,
    pub(super) id: u32,
    pub(super) done: bool,
}

#[cfg(feature = "experimental-build-search")]
#[inline(always)]
pub(super) fn insert_build_candidate(
    pool: &mut Vec<Candidate>,
    candidate: Candidate,
    limit: usize,
) -> Option<usize> {
    debug_assert!(limit > 0 && pool.len() <= limit);
    if pool.len() == limit && candidate.dist >= pool[limit - 1].dist {
        return None;
    }
    let pos = pool.partition_point(|c| c.dist <= candidate.dist);
    if pool.len() == limit {
        // The bound check guarantees that this tail cannot survive the insertion.
        let _ = pool.pop();
    }
    pool.insert(pos, candidate);
    Some(pos)
}

/// Visited marks with an 8-bit epoch, as the vertex index keeps them: a quarter of the cache
/// footprint of `u32` epochs, at the cost of clearing every 255 searches.
pub(super) struct Visited {
    pub(super) marks: Vec<u8>,
    pub(super) epoch: u8,
}

impl Visited {
    pub(super) fn new(n: usize) -> Self {
        Self {
            marks: vec![0; n],
            epoch: 0,
        }
    }

    pub(super) fn next(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.epoch = 1;
        }
    }

    /// True if `id` was not yet visited in this epoch.
    pub(super) fn insert(&mut self, id: u32) -> bool {
        let m = &mut self.marks[id as usize];
        let fresh = *m != self.epoch;
        *m = self.epoch;
        fresh
    }
}

/// One bit per vertex; reset clears only words touched by the previous search. Also holds the
/// construction search's candidate buffers, so a thread allocates them once.
#[derive(Default)]
pub(super) struct BuildVisited {
    pub(super) bits: Vec<u64>,
    pub(super) touched: Vec<u32>,
    pub(super) pool: Vec<Candidate>,
    pub(super) fresh: Vec<u32>,
    pub(super) dists: Vec<f32>,
}

impl BuildVisited {
    pub(super) fn reset(&mut self, n: usize) {
        // Clear before shrinking: scratch is reused across graph sizes and routing layers.
        for word in self.touched.drain(..) {
            self.bits[word as usize] = 0;
        }
        self.bits.resize(n.div_ceil(64), 0);
    }

    /// True if `id` was not yet visited since the last reset.
    pub(super) fn insert(&mut self, id: u32) -> bool {
        let (w, b) = (id as usize / 64, 1u64 << (id % 64));
        let old = self.bits[w];
        if old & b != 0 {
            return false;
        }
        if old == 0 {
            self.touched.push(w as u32);
        }
        self.bits[w] = old | b;
        true
    }
}

/// Profiling only: enables the construction profile and calls `hook` with each phase's name
/// (`setup`, `pass`, `pass`, `fill`, `upper`) as the phase ends, so a profiler can attribute
/// its own counters (allocations, peak bytes) to the phases. Set it before the first build; it
/// can be set once.
#[cfg(feature = "experimental")]
#[doc(hidden)]
pub fn set_build_phase_hook(hook: fn(&str)) -> bool {
    prof::HOOK.set(hook).is_ok()
}
