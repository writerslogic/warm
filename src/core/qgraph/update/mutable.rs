// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Stage 1 of the in-place mutable base (see `docs/L10-STAGE1.md`): one graph over
//! capacity-sized slot arrays, mutated in place. Inserts take the lowest free slots and link
//! them with the build's insertion pass; deletes tombstone a slot that keeps routing; a repair
//! scan splices dead slots out of the lists that name them and only then releases the slots.
//! In memory only, no log, no checkpoint, and independent of [`super::UpdatableVGraph`].

use std::cell::RefCell;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};

use rayon::prelude::*;

use super::super::{
    adjacency::Adjacency, kernels::dot, medoid_by, norms_opt, prepare_rows, BuildCodes,
    BuildParams, BuildVisited, Metric, Rows, BUILD_CODES_MIN_DIM, NONE,
};
use super::{validate_vector, UpdateError};

thread_local! { static SEEN: RefCell<BuildVisited> = RefCell::new(BuildVisited::default()); }

/// Slots per repair block: bounds the repaired lists held before they are applied.
const SCAN_BLOCK: usize = 1 << 14;
/// Ids per `id_to_slot` page (2^16 entries of `u32`, allocated on first use).
const PAGE_BITS: u32 = 16;

/// Knobs of a [`MutableBase`], named as in section 2.7 of the design note.
#[derive(Clone, Debug)]
pub struct MutableOptions {
    /// The most points the caller will hold live at once (`max_pts`).
    pub max_pts: usize,
    /// Extra slots as a fraction of `max_pts`; capacity is `ceil(max_pts * (1 + slack))`.
    pub slack: f64,
    /// Run a repair scan after a delete once dead slots are at least this fraction of the
    /// allocated (live plus dead) slots. 0 scans only when an insert needs slots.
    pub dead_trigger: f64,
    /// In-neighbour recovery on delete: list size of the search for the deleted vector.
    /// 0 is off; other values are rejected until the bounded recovery exists.
    pub repair_ld: usize,
    /// In-neighbour recovery: live out-neighbours of the deleted slot added per hit.
    pub repair_c: usize,
    /// Screen construction distances with 8-bit codes. `None` follows the build: on from
    /// 128 dimensions.
    pub build_codes: Option<bool>,
}

impl MutableOptions {
    /// Capacity for `max_pts` points, no slack, scans only when slots run out.
    pub fn new(max_pts: usize) -> Self {
        Self {
            max_pts,
            slack: 0.0,
            dead_trigger: 0.0,
            repair_ld: 0,
            repair_c: 3,
            build_codes: None,
        }
    }
}

/// What one repair scan did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Live lists that named a dead slot and were pruned again.
    pub repaired: usize,
    /// Dead slots pushed to the free list.
    pub freed: usize,
    /// Live slots that no list reached after the repair and were attached to the routing tree.
    pub orphans: usize,
    /// The entry was dead and a new one was elected.
    pub entry_changed: bool,
}

/// Slot accounting and cumulative scan work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MutableStats {
    pub capacity: usize,
    pub live: usize,
    pub dead: usize,
    pub free: usize,
    pub scans: usize,
    pub repaired_rows: usize,
    pub freed_slots: usize,
    pub orphans: usize,
}

/// Two-level direct table from external id to slot: pages of `2^16` entries, allocated when
/// an id in them is first set.
struct IdTable {
    pages: Vec<Option<Box<[u32]>>>,
}

impl IdTable {
    fn new() -> Self {
        Self {
            pages: (0..1usize << (32 - PAGE_BITS)).map(|_| None).collect(),
        }
    }

    fn get(&self, id: u32) -> u32 {
        match &self.pages[(id >> PAGE_BITS) as usize] {
            Some(page) => page[(id & ((1 << PAGE_BITS) - 1)) as usize],
            None => NONE,
        }
    }

    fn set(&mut self, id: u32, slot: u32) {
        let page = self.pages[(id >> PAGE_BITS) as usize]
            .get_or_insert_with(|| vec![NONE; 1 << PAGE_BITS].into_boxed_slice());
        page[(id & ((1 << PAGE_BITS) - 1)) as usize] = slot;
    }

    fn clear(&mut self, id: u32) {
        if let Some(page) = &mut self.pages[(id >> PAGE_BITS) as usize] {
            page[(id & ((1 << PAGE_BITS) - 1)) as usize] = NONE;
        }
    }
}

/// A score with its slot, ordered by score and then slot.
#[derive(Clone, Copy)]
struct Scored(f32, u32);

impl PartialEq for Scored {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Scored {}
impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0).then(self.1.cmp(&other.1))
    }
}

/// A graph over `capacity` slots that is mutated in place.
///
/// A slot is live (it holds an id), dead (deleted but still allocated, still routing) or free
/// (reusable). Lists may name dead slots between scans and never name free slots.
pub struct MutableBase {
    dim: usize,
    params: BuildParams,
    dead_trigger: f64,
    capacity: usize,
    rows: Vec<f32>,
    /// Squared norm per slot (Euclidean); empty for unit rows.
    norms: Vec<f32>,
    codes: Option<BuildCodes>,
    adj: Adjacency,
    /// The id of a live slot, `NONE` for a dead or free slot.
    slot_to_id: Vec<u32>,
    id_to_slot: IdTable,
    /// Tombstone bitset over slots.
    dead: Vec<u64>,
    /// Freed slots, lowest first. Every slot from `next_fresh` up is free as well.
    free: BinaryHeap<Reverse<u32>>,
    next_fresh: usize,
    /// Routing tree rooted at `entry`: keeps every allocated slot reachable when reciprocal
    /// pruning removes its last incoming list edge.
    first_child: Vec<u32>,
    next_sibling: Vec<u32>,
    entry: u32,
    live: usize,
    dead_count: usize,
    totals: MutableStats,
}

impl MutableBase {
    /// An empty index of `options.max_pts * (1 + slack)` slots of `dim` dimensions.
    pub fn new(
        dim: usize,
        params: &BuildParams,
        options: &MutableOptions,
    ) -> Result<Self, UpdateError> {
        let invalid = |what: &str| UpdateError::Search(super::super::invalid(what.to_owned()));
        if dim == 0 || options.max_pts == 0 {
            return Err(invalid("dimension and max_pts must be positive"));
        }
        if params.degree == 0
            || params.build_ef == 0
            || !(params.alpha.is_finite() && params.alpha >= 1.0)
        {
            return Err(invalid(
                "degree and build_ef must be positive and alpha at least 1",
            ));
        }
        if !(options.slack.is_finite() && options.slack >= 0.0) {
            return Err(invalid("slack must be finite and non-negative"));
        }
        if !(0.0..=1.0).contains(&options.dead_trigger) {
            return Err(invalid("dead_trigger must be in [0, 1]"));
        }
        if options.repair_ld != 0 {
            return Err(UpdateError::UnsupportedFormat(
                "delete-time in-neighbour repair (repair_ld) is not implemented in stage 1",
            ));
        }
        let scaled = (options.max_pts as f64 * (1.0 + options.slack)).ceil();
        // Slot ids are `u32` below the padding value `NONE`.
        if !scaled.is_finite() || scaled >= f64::from(NONE) {
            return Err(UpdateError::Capacity);
        }
        let capacity = scaled as usize;
        capacity
            .checked_mul(dim.max(params.degree))
            .ok_or(UpdateError::Capacity)?;
        let euclidean = params.metric == Metric::Euclidean;
        let codes = options
            .build_codes
            .unwrap_or(dim >= BUILD_CODES_MIN_DIM)
            .then(|| BuildCodes::with_rows(capacity, dim, euclidean));
        Ok(Self {
            dim,
            params: params.clone(),
            dead_trigger: options.dead_trigger,
            capacity,
            rows: vec![0.0; capacity * dim],
            norms: if euclidean {
                vec![0.0; capacity]
            } else {
                Vec::new()
            },
            codes,
            adj: Adjacency::new(capacity, params.degree),
            slot_to_id: vec![NONE; capacity],
            id_to_slot: IdTable::new(),
            dead: vec![0; capacity.div_ceil(64)],
            free: BinaryHeap::new(),
            next_fresh: 0,
            first_child: vec![NONE; capacity],
            next_sibling: vec![NONE; capacity],
            entry: NONE,
            live: 0,
            dead_count: 0,
            totals: MutableStats::default(),
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn metric(&self) -> Metric {
        self.params.metric
    }

    /// Live points.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Whether `id` is live.
    pub fn contains(&self, id: u32) -> bool {
        id != NONE && self.id_to_slot.get(id) != NONE
    }

    pub fn stats(&self) -> MutableStats {
        MutableStats {
            capacity: self.capacity,
            live: self.live,
            dead: self.dead_count,
            free: self.free_count(),
            ..self.totals
        }
    }

    fn free_count(&self) -> usize {
        self.free.len() + (self.capacity - self.next_fresh)
    }

    fn is_dead(&self, slot: u32) -> bool {
        self.dead[slot as usize / 64] & (1 << (slot % 64)) != 0
    }

    fn is_live(&self, slot: u32) -> bool {
        self.slot_to_id[slot as usize] != NONE
    }

    fn take_slot(&mut self) -> u32 {
        if let Some(Reverse(slot)) = self.free.pop() {
            return slot;
        }
        self.next_fresh += 1;
        (self.next_fresh - 1) as u32
    }

    fn link_child(&mut self, parent: u32, child: u32) {
        self.next_sibling[child as usize] = self.first_child[parent as usize];
        self.first_child[parent as usize] = child;
    }

    /// Adds `vectors` (row-major) under `ids`, all or none. Takes the lowest free slots,
    /// running a repair scan first when there are too few, then links the new slots with the
    /// build's insertion pass over the live and dead graph.
    pub fn insert_batch(&mut self, ids: &[u32], vectors: &[f32]) -> Result<(), UpdateError> {
        let dim = self.dim;
        let expected = ids.len().checked_mul(dim).ok_or(UpdateError::Capacity)?;
        if vectors.len() != expected {
            return Err(UpdateError::DimensionMismatch {
                expected,
                actual: vectors.len(),
            });
        }
        if ids.is_empty() {
            return Ok(());
        }
        let mut batch = HashSet::with_capacity(ids.len());
        for (&id, row) in ids.iter().zip(vectors.chunks_exact(dim)) {
            if id == NONE {
                return Err(UpdateError::IdOutOfRange);
            }
            if !batch.insert(id) {
                return Err(UpdateError::DuplicateIdInBatch(id));
            }
            if self.contains(id) {
                return Err(UpdateError::AlreadyExists(id));
            }
            validate_vector(row, dim, self.params.metric)?;
        }
        let need = ids.len();
        if need > self.free_count() + self.dead_count {
            return Err(UpdateError::Capacity);
        }
        // With no live slot there is nothing to keep routing: release the dead ones so the
        // new rows start a fresh graph.
        if self.free_count() < need || (self.live == 0 && self.dead_count > 0) {
            self.repair_scan();
        }
        let slots: Vec<u32> = (0..need).map(|_| self.take_slot()).collect();
        let euclidean = self.params.metric == Metric::Euclidean;
        let mut prepared = vectors.to_vec();
        let norms = prepare_rows(&mut prepared, dim, self.params.metric);
        for (i, (&id, &slot)) in ids.iter().zip(&slots).enumerate() {
            let s = slot as usize;
            let row = &prepared[i * dim..(i + 1) * dim];
            self.rows[s * dim..(s + 1) * dim].copy_from_slice(row);
            if euclidean {
                self.norms[s] = norms[i];
            }
            if let Some(codes) = &mut self.codes {
                codes.set_row(s, row, euclidean.then(|| norms[i]));
            }
            self.slot_to_id[s] = id;
            self.id_to_slot.set(id, slot);
            self.first_child[s] = NONE;
            self.next_sibling[s] = NONE;
        }
        if self.live == 0 {
            self.entry = slots[0];
        }
        self.live += need;
        let rows = Rows {
            data: &self.rows,
            dim,
            norms: norms_opt(&self.norms),
            codes: self.codes.as_ref(),
            quantized: false,
        };
        rows.insert_pass(
            &mut self.adj,
            &slots,
            self.entry,
            self.params.build_ef,
            self.params.alpha,
            self.params.degree,
        );
        // A parent that was in the graph before this slot keeps the tree acyclic and rooted at
        // the entry; inside the batch only an earlier slot qualifies.
        let position: HashMap<u32, usize> =
            slots.iter().enumerate().map(|(i, &s)| (s, i)).collect();
        for (j, &slot) in slots.iter().enumerate() {
            if slot == self.entry {
                continue;
            }
            let parent = self
                .adj
                .get(slot as usize)
                .iter()
                .copied()
                .find(|u| position.get(u).is_none_or(|&k| k < j))
                .unwrap_or(self.entry);
            self.link_child(parent, slot);
        }
        Ok(())
    }

    /// Deletes `ids`, all or none: each slot is tombstoned and keeps routing until a scan.
    pub fn delete_batch(&mut self, ids: &[u32]) -> Result<(), UpdateError> {
        let mut batch = HashSet::with_capacity(ids.len());
        for &id in ids {
            if !batch.insert(id) {
                return Err(UpdateError::DuplicateIdInBatch(id));
            }
            if !self.contains(id) {
                return Err(UpdateError::UnknownId(id));
            }
        }
        for &id in ids {
            let slot = self.id_to_slot.get(id);
            self.id_to_slot.clear(id);
            self.slot_to_id[slot as usize] = NONE;
            self.dead[slot as usize / 64] |= 1 << (slot % 64);
        }
        self.live -= ids.len();
        self.dead_count += ids.len();
        if self.dead_trigger > 0.0
            && self.dead_count > 0
            && self.dead_count as f64 >= self.dead_trigger * (self.live + self.dead_count) as f64
        {
            self.repair_scan();
        }
        Ok(())
    }

    pub fn delete(&mut self, id: u32) -> Result<(), UpdateError> {
        self.delete_batch(&[id])
    }

    fn score(&self, query: &[f32], slot: u32) -> f32 {
        let s = slot as usize;
        let row = &self.rows[s * self.dim..(s + 1) * self.dim];
        match self.params.metric {
            Metric::Angular => -dot(query, row),
            Metric::Euclidean => self.norms[s] * 0.5 - dot(query, row),
        }
    }

    /// Best-first search from the entry over list and routing-tree edges. Dead slots are
    /// traversed and expanded like live ones; only live slots enter the result set, which
    /// holds the `ef` best live slots seen. Ascending by score, then slot.
    fn search_slots(&self, query: &[f32], ef: usize) -> Vec<Scored> {
        if self.live == 0 {
            return Vec::new();
        }
        SEEN.with(|seen| {
            let seen = &mut *seen.borrow_mut();
            seen.reset(self.capacity);
            seen.insert(self.entry);
            let first = Scored(self.score(query, self.entry), self.entry);
            let mut candidates = BinaryHeap::from([Reverse(first)]);
            let mut results = BinaryHeap::new();
            if self.is_live(self.entry) {
                results.push(first);
            }
            while let Some(Reverse(current)) = candidates.pop() {
                if results.len() >= ef && results.peek().is_some_and(|worst| current > *worst) {
                    break;
                }
                let at = current.1 as usize;
                let children = std::iter::successors(
                    Some(self.first_child[at]).filter(|&c| c != NONE),
                    |&c| Some(self.next_sibling[c as usize]).filter(|&c| c != NONE),
                );
                for u in self.adj.get(at).iter().copied().chain(children) {
                    if !seen.insert(u) {
                        continue;
                    }
                    let scored = Scored(self.score(query, u), u);
                    if results.len() >= ef && results.peek().is_some_and(|worst| scored > *worst) {
                        continue;
                    }
                    candidates.push(Reverse(scored));
                    if self.is_live(u) {
                        results.push(scored);
                        if results.len() > ef {
                            results.pop();
                        }
                    }
                }
            }
            results.into_sorted_vec()
        })
    }

    /// The `k` best live points for `query` as `(score, id)`, ascending, from a result set of
    /// `max(ef, k)` live slots. The score is `-<q, x>` for angular rows and `|x|^2 / 2 - <q, x>`
    /// for Euclidean rows, computed on the f32 rows.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
    ) -> Result<Vec<(f32, u32)>, UpdateError> {
        if query.len() != self.dim {
            return Err(UpdateError::DimensionMismatch {
                expected: self.dim,
                actual: query.len(),
            });
        }
        if k == 0 {
            return Ok(Vec::new());
        }
        let found = self.search_slots(query, ef.max(k));
        Ok(found
            .into_iter()
            .take(k)
            .map(|Scored(d, slot)| (d, self.slot_to_id[slot as usize]))
            .collect())
    }

    /// The repaired list of live slot `s`, or `None` when it names no dead slot: its live
    /// neighbours plus the live out-neighbours of its dead neighbours, pruned once.
    fn repaired_list(&self, rows: &Rows<'_>, s: u32) -> Option<Vec<u32>> {
        if !self.is_live(s) {
            return None;
        }
        let list = self.adj.get(s as usize);
        if !list.iter().any(|&u| self.is_dead(u)) {
            return None;
        }
        let mut ids = Vec::with_capacity(list.len() * 2);
        for &u in list {
            if self.is_dead(u) {
                ids.extend(
                    self.adj
                        .get(u as usize)
                        .iter()
                        .copied()
                        .filter(|&w| w != s && self.is_live(w)),
                );
            } else {
                ids.push(u);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let mut d = Vec::new();
        rows.d2_stream(s, &ids, &mut d);
        let mut cands: Vec<(f32, u32)> = d.into_iter().zip(ids).collect();
        let mut out = Vec::with_capacity(self.params.degree);
        rows.prune_into(
            s,
            &mut cands,
            self.params.alpha,
            self.params.degree,
            &mut out,
        );
        Some(out)
    }

    /// Repairs every live list that names a dead slot, then releases the dead slots to the
    /// free list. A slot is released only after every list has been repaired, so no list ever
    /// names a free slot. Then the entry is re-elected if it was dead and the routing tree is
    /// rebuilt over the live slots.
    pub fn repair_scan(&mut self) -> ScanStats {
        let mut stats = ScanStats::default();
        if self.dead_count == 0 {
            return stats;
        }
        let high = self.next_fresh;
        for base in (0..high).step_by(SCAN_BLOCK) {
            let end = (base + SCAN_BLOCK).min(high);
            // Candidates read only dead slots' lists and ids, none of which a repair writes,
            // so a block's result does not depend on the blocks applied before it.
            let fixed: Vec<(u32, Vec<u32>)> = {
                let rows = Rows {
                    data: &self.rows,
                    dim: self.dim,
                    norms: norms_opt(&self.norms),
                    codes: self.codes.as_ref(),
                    quantized: false,
                };
                (base as u32..end as u32)
                    .into_par_iter()
                    .filter_map(|s| self.repaired_list(&rows, s).map(|list| (s, list)))
                    .collect()
            };
            stats.repaired += fixed.len();
            for (s, list) in &fixed {
                self.adj.set(*s as usize, list);
            }
        }
        if self.live == 0 {
            self.entry = NONE;
        } else if self.is_dead(self.entry) {
            let live: Vec<u32> = (0..high as u32).filter(|&s| self.is_live(s)).collect();
            let rows = Rows {
                data: &self.rows,
                dim: self.dim,
                norms: norms_opt(&self.norms),
                codes: None,
                quantized: false,
            };
            let at = medoid_by(&rows, live.len(), |i| live[i as usize], self.params.metric);
            self.entry = live[at as usize];
            stats.entry_changed = true;
        }
        for s in 0..high as u32 {
            if self.is_dead(s) {
                self.adj.set(s as usize, &[]);
                self.dead[s as usize / 64] &= !(1 << (s % 64));
                self.free.push(Reverse(s));
                stats.freed += 1;
            }
        }
        self.dead_count = 0;
        stats.orphans = self.rebuild_tree();
        self.totals.scans += 1;
        self.totals.repaired_rows += stats.repaired;
        self.totals.freed_slots += stats.freed;
        self.totals.orphans += stats.orphans;
        #[cfg(debug_assertions)]
        if let Err(violation) = self.verify(true) {
            panic!("repair scan broke an invariant: {violation}");
        }
        stats
    }

    /// Breadth-first from `queue`, giving each newly reached slot the slot it was reached from
    /// as its routing parent.
    fn extend_tree(&mut self, queue: &mut Vec<u32>, reached: &mut [bool]) {
        let mut head = 0;
        while head < queue.len() {
            let v = queue[head];
            head += 1;
            for i in 0..self.adj.get(v as usize).len() {
                let u = self.adj.get(v as usize)[i];
                if !reached[u as usize] {
                    reached[u as usize] = true;
                    self.link_child(v, u);
                    queue.push(u);
                }
            }
        }
    }

    /// Rebuilds the routing tree as a breadth-first spanning tree of the lists from the entry.
    /// A live slot the lists do not reach is searched for like a new row and attached under
    /// the nearest slot the search reaches. Returns the number of such slots.
    fn rebuild_tree(&mut self) -> usize {
        let high = self.next_fresh;
        self.first_child[..high].fill(NONE);
        self.next_sibling[..high].fill(NONE);
        if self.live == 0 {
            return 0;
        }
        let mut reached = vec![false; high];
        reached[self.entry as usize] = true;
        let mut queue = vec![self.entry];
        self.extend_tree(&mut queue, &mut reached);
        let mut orphans = 0;
        for s in 0..high as u32 {
            if reached[s as usize] || !self.is_live(s) {
                continue;
            }
            orphans += 1;
            let at = s as usize * self.dim;
            let row = self.rows[at..at + self.dim].to_vec();
            let parent = self
                .search_slots(&row, self.params.build_ef)
                .into_iter()
                .map(|Scored(_, slot)| slot)
                .find(|&slot| slot != s)
                .unwrap_or(self.entry);
            reached[s as usize] = true;
            self.link_child(parent, s);
            queue.clear();
            queue.push(s);
            self.extend_tree(&mut queue, &mut reached);
        }
        orphans
    }

    /// Checks the invariants of the design note: I1 (degree, no self edge, no duplicate, ids in
    /// range), I2 (no list names a free slot; after a scan, `after_scan`, none names a dead
    /// slot either), I3 (every live slot is reachable from the entry over list and routing
    /// edges), I4 (the entry is allocated, and live after a scan) and I6 (`id_to_slot` and
    /// `slot_to_id` are inverse; dead and free slots hold no id). Costs O(capacity * degree).
    pub fn verify(&self, after_scan: bool) -> Result<(), String> {
        let high = self.next_fresh;
        let r = self.params.degree;
        let (mut live, mut dead) = (0, 0);
        for s in 0..high as u32 {
            let (is_live, is_dead) = (self.is_live(s), self.is_dead(s));
            if is_live && is_dead {
                return Err(format!("slot {s} is live and dead"));
            }
            let id = self.slot_to_id[s as usize];
            if is_live {
                live += 1;
                if self.id_to_slot.get(id) != s {
                    return Err(format!(
                        "I6: slot {s} holds id {id} but the table says otherwise"
                    ));
                }
            } else if is_dead {
                dead += 1;
            }
            let list = self.adj.get(s as usize);
            if !is_live && !is_dead && !list.is_empty() {
                return Err(format!("I1: free slot {s} has a list"));
            }
            if list.len() > r {
                return Err(format!("I1: slot {s} has {} edges, degree {r}", list.len()));
            }
            let mut seen = HashSet::with_capacity(list.len());
            for &u in list {
                if u as usize >= high || u == s || !seen.insert(u) {
                    return Err(format!(
                        "I1: slot {s} has a self, duplicate or out-of-range edge {u}"
                    ));
                }
                if !self.is_live(u) && !self.is_dead(u) {
                    return Err(format!("I2: slot {s} names free slot {u}"));
                }
                if after_scan && !is_live && !list.is_empty() {
                    return Err(format!("I2: slot {s} is not live and keeps a list"));
                }
                if after_scan && is_live && !self.is_live(u) {
                    return Err(format!(
                        "I2: live slot {s} names dead slot {u} after a scan"
                    ));
                }
            }
        }
        for s in high..self.capacity {
            if self.is_live(s as u32) || self.is_dead(s as u32) {
                return Err(format!("slot {s} is in use above the high-water mark"));
            }
        }
        if live != self.live || dead != self.dead_count {
            return Err(format!(
                "counts: live {live} vs {}, dead {dead} vs {}",
                self.live, self.dead_count
            ));
        }
        if live + dead + self.free_count() != self.capacity {
            return Err("live + dead + free is not the capacity".into());
        }
        let mut freed: Vec<u32> = self.free.iter().map(|r| r.0).collect();
        freed.sort_unstable();
        if freed.windows(2).any(|w| w[0] == w[1])
            || freed
                .iter()
                .any(|&s| s as usize >= high || self.is_live(s) || self.is_dead(s))
        {
            return Err("free list holds a duplicate or an allocated slot".into());
        }
        if self.live == 0 {
            return Ok(());
        }
        let entry = self.entry;
        if entry as usize >= high || (!self.is_live(entry) && !self.is_dead(entry)) {
            return Err(format!("I4: entry {entry} is not allocated"));
        }
        if after_scan && !self.is_live(entry) {
            return Err(format!("I4: entry {entry} is dead after a scan"));
        }
        let mut reached = vec![false; high];
        reached[entry as usize] = true;
        let mut stack = vec![entry];
        while let Some(v) = stack.pop() {
            let children = std::iter::successors(
                Some(self.first_child[v as usize]).filter(|&c| c != NONE),
                |&c| Some(self.next_sibling[c as usize]).filter(|&c| c != NONE),
            );
            for u in self.adj.get(v as usize).iter().copied().chain(children) {
                if u as usize >= high || (!self.is_live(u) && !self.is_dead(u)) {
                    return Err(format!("I3: edge from {v} to unallocated slot {u}"));
                }
                if !std::mem::replace(&mut reached[u as usize], true) {
                    stack.push(u);
                }
            }
        }
        match (0..high as u32).find(|&s| self.is_live(s) && !reached[s as usize]) {
            Some(s) => Err(format!(
                "I3: live slot {s} is unreachable from entry {entry}"
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
