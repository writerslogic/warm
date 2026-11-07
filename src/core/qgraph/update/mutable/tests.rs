// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use super::super::super::kernels::SplitMix;
use super::*;

fn params(metric: Metric) -> BuildParams {
    BuildParams {
        metric,
        degree: 16,
        build_ef: 48,
        ..Default::default()
    }
}

/// An index and a shadow of what it must hold, driven by a seeded op sequence.
struct Model {
    index: MutableBase,
    /// Prepared (angular: normalized) rows of the live ids.
    shadow: BTreeMap<u32, Vec<f32>>,
    rng: SplitMix,
    next_id: u32,
    recycled: Vec<u32>,
    centers: Vec<Vec<f32>>,
    metric: Metric,
    dim: usize,
    cap: usize,
}

impl Model {
    fn new(metric: Metric, dim: usize, cap: usize, seed: u64, codes: Option<bool>) -> Self {
        let mut rng = SplitMix(seed);
        let centers = (0..24)
            .map(|_| (0..dim).map(|_| rng.next_f32() * 2.0 - 1.0).collect())
            .collect();
        let mut options = MutableOptions::new(cap);
        options.build_codes = codes;
        Self {
            index: MutableBase::new(dim, &params(metric), &options).unwrap(),
            shadow: BTreeMap::new(),
            rng,
            next_id: 0,
            recycled: Vec::new(),
            centers,
            metric,
            dim,
            cap,
        }
    }

    fn below(&mut self, n: usize) -> usize {
        (self.rng.next_u64() % n as u64) as usize
    }

    fn row(&mut self) -> Vec<f32> {
        let c = self.below(self.centers.len());
        (0..self.dim)
            .map(|i| self.centers[c][i] + (self.rng.next_f32() - 0.5) * 0.5)
            .collect()
    }

    fn prepared(&self, raw: &[f32]) -> Vec<f32> {
        let mut row = raw.to_vec();
        prepare_rows(&mut row, self.dim, self.metric);
        row
    }

    fn insert(&mut self, count: usize) {
        let mut ids = Vec::new();
        let mut flat = Vec::new();
        for _ in 0..count {
            let id = if !self.recycled.is_empty() && self.below(3) == 0 {
                let at = self.below(self.recycled.len());
                self.recycled.swap_remove(at)
            } else {
                self.next_id += 1;
                self.next_id
            };
            let raw = self.row();
            self.shadow.insert(id, self.prepared(&raw));
            ids.push(id);
            flat.extend(raw);
        }
        self.index.insert_batch(&ids, &flat).unwrap();
    }

    fn delete(&mut self, count: usize) {
        let mut keys: Vec<u32> = self.shadow.keys().copied().collect();
        let count = count.min(keys.len());
        for i in 0..count {
            let j = i + self.below(keys.len() - i);
            keys.swap(i, j);
        }
        keys.truncate(count);
        self.index.delete_batch(&keys).unwrap();
        for id in &keys {
            self.shadow.remove(id);
        }
        self.recycled.extend(keys);
    }

    fn step(&mut self) {
        let room = self.cap - self.shadow.len();
        match self.below(20) {
            0 => {
                self.index.repair_scan();
                self.check(true);
            }
            1..=11 if room > 0 => {
                let n = (1 + self.below(200)).min(room);
                self.insert(n);
                self.check(false);
            }
            _ => {
                let n = 1 + self.below(300);
                self.delete(n);
                self.check(false);
            }
        }
    }

    fn check(&self, after_scan: bool) {
        self.index.verify(after_scan).unwrap();
        assert_eq!(self.index.len(), self.shadow.len());
    }

    fn check_contents(&self) {
        for (&id, row) in &self.shadow {
            let slot = self.index.id_to_slot.get(id) as usize;
            assert_eq!(self.index.slot_to_id[slot], id);
            assert_eq!(
                &self.index.rows[slot * self.dim..(slot + 1) * self.dim],
                row
            );
        }
    }

    fn score(&self, q: &[f32], row: &[f32]) -> f32 {
        match self.metric {
            Metric::Angular => -dot(q, row),
            Metric::Euclidean => dot(row, row) * 0.5 - dot(q, row),
        }
    }

    fn exact(&self, q: &[f32], k: usize) -> Vec<u32> {
        let mut all: Vec<(f32, u32)> = self
            .shadow
            .iter()
            .map(|(&id, row)| (self.score(q, row), id))
            .collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        all.into_iter().take(k).map(|x| x.1).collect()
    }

    /// Recall@k of the index against brute force over the live set, and the invariants every
    /// answer must keep at any ef: distinct live ids, ascending scores equal to the exact ones.
    fn recall(&mut self, queries: usize, k: usize, ef: usize) -> f64 {
        let queries = self.queries(queries);
        self.recall_on(&queries, k, ef)
    }

    fn queries(&mut self, n: usize) -> Vec<Vec<f32>> {
        (0..n).map(|_| self.row()).collect()
    }

    fn recall_on(&self, queries: &[Vec<f32>], k: usize, ef: usize) -> f64 {
        let mut hit = 0;
        for q in queries {
            let got = self.index.search(q, k, ef).unwrap();
            let ids: Vec<u32> = got.iter().map(|g| g.1).collect();
            assert_eq!(got.len(), k.min(self.shadow.len()));
            assert!(got.windows(2).all(|w| w[0].0 <= w[1].0));
            assert!(ids.iter().all(|id| self.shadow.contains_key(id)));
            assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
            for g in &got {
                assert_eq!(g.0, self.score(q, &self.shadow[&g.1]));
            }
            let want = self.exact(q, k);
            hit += want.iter().filter(|id| ids.contains(id)).count();
        }
        hit as f64 / (queries.len() * k.min(self.shadow.len())) as f64
    }
}

fn random_op_sequence(metric: Metric, dim: usize) {
    let cap = 5000;
    let mut m = Model::new(metric, dim, cap, 20261107, None);
    while m.shadow.len() < cap - 400 {
        m.insert(400);
        m.check(false);
    }
    for step in 0..150 {
        m.step();
        if step % 30 == 29 {
            // Dead slots may be present: an exhaustive search must still find every live
            // point and no dead one, which also shows every live slot is reachable.
            assert_eq!(m.recall(5, 10, cap), 1.0, "step {step}");
        }
    }
    m.check_contents();
    let stats = m.index.stats();
    assert!(stats.scans >= 3, "{stats:?}");
    assert!(stats.repaired_rows > 0, "{stats:?}");
    let queries = m.queries(100);
    for ef in [32, 64, 128] {
        let recall = m.recall_on(&queries, 10, ef);
        println!(
            "{metric:?} dim={dim} live={} recall@10 ef={ef}: {recall:.4}",
            m.shadow.len()
        );
    }
    m.index.repair_scan();
    m.check(true);
    m.check_contents();
    assert_eq!(m.recall(5, 10, cap), 1.0);
    for ef in [32, 64, 128] {
        let recall = m.recall_on(&queries, 10, ef);
        println!("{metric:?} dim={dim} after scan recall@10 ef={ef}: {recall:.4}");
    }
    println!("{metric:?} dim={dim} stats {:?}", m.index.stats());
}

#[test]
fn random_ops_keep_invariants_euclidean() {
    random_op_sequence(Metric::Euclidean, 16);
}

#[test]
fn random_ops_keep_invariants_angular() {
    random_op_sequence(Metric::Angular, 32);
}

fn fingerprint(m: &Model) -> (Adjacency, Vec<u32>, Vec<u32>, u32, Vec<u32>, Vec<u32>) {
    let mut free: Vec<u32> = m.index.free.iter().map(|r| r.0).collect();
    free.sort_unstable();
    (
        m.index.adj.clone(),
        m.index.slot_to_id.clone(),
        free,
        m.index.entry,
        m.index.first_child.clone(),
        m.index.next_sibling.clone(),
    )
}

#[test]
fn replay_is_deterministic_and_codes_leave_the_graph_unchanged() {
    let run = |codes| {
        let mut m = Model::new(Metric::Euclidean, 16, 600, 99, Some(codes));
        while m.shadow.len() < 500 {
            m.insert(100);
        }
        for _ in 0..40 {
            m.step();
        }
        m.index.repair_scan();
        m.check(true);
        fingerprint(&m)
    };
    let plain = run(false);
    assert!(plain == run(false), "same ops, different state");
    assert!(plain == run(true), "codes changed the graph");
}

#[test]
fn freed_slots_are_reused_lowest_first_after_the_scan() {
    let mut m = Model::new(Metric::Euclidean, 16, 64, 5, None);
    m.insert(20);
    let slot_of = |m: &Model, id: u32| m.index.id_to_slot.get(id);
    let victims: Vec<u32> = [3u32, 7, 11, 2].to_vec();
    let mut want: Vec<u32> = victims.iter().map(|&id| slot_of(&m, id)).collect();
    want.sort_unstable();
    m.delete_ids(&victims);
    // Dead slots are not reusable before a scan.
    assert_eq!(m.index.stats().free, 64 - 20);
    assert_eq!(m.index.stats().dead, 4);
    let stats = m.index.repair_scan();
    assert_eq!(stats.freed, 4);
    m.check(true);
    m.insert_ids(&[100, 101, 102, 103, 104, 105]);
    let got: Vec<u32> = (100..106).map(|id| slot_of(&m, id)).collect();
    let mut expect = want.clone();
    expect.extend([20, 21]);
    assert_eq!(got, expect);
    m.check(false);
}

impl Model {
    fn delete_ids(&mut self, ids: &[u32]) {
        self.index.delete_batch(ids).unwrap();
        for id in ids {
            self.shadow.remove(id);
        }
    }

    fn insert_ids(&mut self, ids: &[u32]) {
        let mut flat = Vec::new();
        for &id in ids {
            let raw = self.row();
            self.shadow.insert(id, self.prepared(&raw));
            flat.extend(raw);
        }
        self.index.insert_batch(ids, &flat).unwrap();
    }
}

#[test]
fn capacity_boundaries_leave_state_unchanged_on_refusal() {
    let mut m = Model::new(Metric::Euclidean, 16, 64, 11, None);
    m.insert_ids(&(0..64).collect::<Vec<_>>());
    // Full: one more is refused.
    let raw = m.row();
    assert!(matches!(
        m.index.insert_batch(&[1000], &raw),
        Err(UpdateError::Capacity)
    ));
    m.check(false);
    assert_eq!(m.index.len(), 64);
    // Three dead slots: limit - 1, limit and limit + 1 of what a scan can release.
    m.delete_ids(&[1, 2, 3]);
    let flat: Vec<f32> = (0..4).flat_map(|_| m.row()).collect();
    assert!(matches!(
        m.index.insert_batch(&[1000, 1001, 1002, 1003], &flat),
        Err(UpdateError::Capacity)
    ));
    assert_eq!(m.index.stats().dead, 3);
    m.check(false);
    m.insert_ids(&[1000, 1001]);
    assert_eq!(m.index.stats().scans, 1);
    m.delete_ids(&[1000, 1001]);
    m.insert_ids(&[2000, 2001, 2002]);
    m.check(false);
    assert_eq!(m.index.len(), 64 - 3 + 2 - 2 + 3);
}

#[test]
fn deleting_everything_then_inserting_starts_a_fresh_graph() {
    let mut m = Model::new(Metric::Angular, 16, 200, 3, None);
    m.insert_ids(&(0..150).collect::<Vec<_>>());
    m.delete_ids(&(0..150).collect::<Vec<_>>());
    assert!(m.index.search(&[1.0; 16], 5, 16).unwrap().is_empty());
    m.insert_ids(&(1000..1100).collect::<Vec<_>>());
    m.check(false);
    assert_eq!(m.index.stats().dead, 0);
    assert_eq!(m.recall(10, 5, 200), 1.0);
}

#[test]
fn dead_trigger_scans_after_the_deleting_batch() {
    let mut options = MutableOptions::new(300);
    options.dead_trigger = 0.25;
    let mut index = MutableBase::new(16, &params(Metric::Euclidean), &options).unwrap();
    let mut rng = SplitMix(1);
    let flat: Vec<f32> = (0..200 * 16).map(|_| rng.next_f32()).collect();
    index
        .insert_batch(&(0..200).collect::<Vec<_>>(), &flat)
        .unwrap();
    index.delete_batch(&(0..49).collect::<Vec<_>>()).unwrap();
    assert_eq!(index.stats().dead, 49);
    index.delete(49).unwrap();
    // 50 dead of 200 allocated is exactly the trigger.
    assert_eq!(index.stats().dead, 0);
    assert_eq!(index.stats().scans, 1);
    index.verify(true).unwrap();
}

#[test]
fn bad_input_is_refused_without_changing_the_index() {
    let mut m = Model::new(Metric::Euclidean, 16, 32, 8, None);
    m.insert_ids(&[1, 2, 3]);
    let row = m.row();
    let two: Vec<f32> = [row.clone(), row.clone()].concat();
    assert!(matches!(
        m.index.insert_batch(&[7, 7], &two),
        Err(UpdateError::DuplicateIdInBatch(7))
    ));
    assert!(matches!(
        m.index.insert_batch(&[2], &row),
        Err(UpdateError::AlreadyExists(2))
    ));
    assert!(matches!(
        m.index.insert_batch(&[u32::MAX], &row),
        Err(UpdateError::IdOutOfRange)
    ));
    assert!(matches!(
        m.index.insert_batch(&[9], &row[..15]),
        Err(UpdateError::DimensionMismatch { .. })
    ));
    assert!(matches!(
        m.index.delete_batch(&[1, 99]),
        Err(UpdateError::UnknownId(99))
    ));
    assert!(m.index.contains(1));
    let mut options = MutableOptions::new(8);
    options.repair_ld = 64;
    assert!(MutableBase::new(16, &params(Metric::Euclidean), &options).is_err());
    m.check(false);
    assert_eq!(m.index.len(), 3);
}
