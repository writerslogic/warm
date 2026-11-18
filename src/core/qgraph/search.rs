// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Per-thread search state for one index.
pub struct Searcher<'a> {
    pub(super) index: &'a QGraph,
    pub(super) visited: Visited,
    pub(super) pq: Vec<f32>,
    pub(super) code: QueryCode,
    pub(super) raw: Vec<u32>,
    pub(super) est: Vec<f32>,
    pub(super) pool: Vec<Candidate>,
    pub(super) results: Vec<(f32, u32)>,
}

impl Searcher<'_> {
    /// Writes the ids of the (approximate) `k` nearest vectors by cosine into `out`, nearest
    /// first, and returns the number of exactly scored vertices.
    pub fn search(
        &mut self,
        query: &[f32],
        k: usize,
        params: SearchParams,
        out: &mut Vec<u32>,
    ) -> usize {
        let idx = self.index;
        let ef = params.ef.max(1);
        idx.rotation.apply(query, &mut self.pq);
        self.code.encode(&self.pq);
        let (lo2, d2q, sq) = (2.0 * self.code.lo, 2.0 * self.code.delta, self.code.sum);

        self.visited.next();
        let (entry, mut exact) = idx.descend(query);
        let mut expanded = 0;
        self.visited.insert(entry);
        self.pool.clear();
        self.results.clear();
        self.pool.push(Candidate {
            dist: 0.0,
            id: entry,
            done: false,
        });
        let mut cur = 0;
        while cur < self.pool.len() {
            if params.max_exact > 0 && expanded >= params.max_exact {
                break;
            }
            let v = self.pool[cur].id;
            if let Some(next) = self.pool[cur + 1..].iter().find(|c| !c.done) {
                kernels::prefetch(idx.block(next.id));
            }
            let b = idx.block(v);
            let ipv = dot(query, &as_f32(&b[..idx.codes_off])[..idx.dim]);
            exact += 1;
            expanded += 1;
            push_top(&mut self.results, k, -ipv, v);
            // The expanded vertex moves to the slot of its exact distance, so an optimistic
            // estimate cannot hold a pool slot and stall the search. The pool is sorted and at
            // most `ef` long, so the slot is the one a remove and re-insert would give, and
            // one rotation of the span between the two slots moves it there.
            let mut pos = self.pool.partition_point(|c| c.dist <= -ipv);
            if pos > cur {
                pos -= 1;
                self.pool[cur..=pos].rotate_left(1);
            } else {
                self.pool[pos..=cur].rotate_right(1);
            }
            self.pool[pos] = Candidate {
                dist: -ipv,
                id: v,
                done: true,
            };

            kernels::raw_estimates(
                &b[idx.codes_off..idx.factors_off],
                &self.code.planes,
                idx.words,
                &mut self.raw,
            );
            let fac = as_f32(&b[idx.factors_off..idx.ids_off]);
            let (kf, rest) = fac.split_at(idx.degree);
            let (mf, pf) = rest.split_at(idx.degree);
            for ((((e, &kj), &mj), &pj), &rj) in
                self.est.iter_mut().zip(kf).zip(mf).zip(pf).zip(&self.raw)
            {
                *e = -(ipv + kj + mj * (lo2 * pj - sq + d2q * rj as f32));
            }

            let mut best = usize::MAX;
            for (&u, &d) in as_u32(&b[idx.ids_off..]).iter().zip(&self.est) {
                if u == NONE || !self.visited.insert(u) {
                    continue;
                }
                if self.pool.len() >= ef && d >= self.pool[ef - 1].dist {
                    continue;
                }
                let pos = self.pool.partition_point(|c| c.dist <= d);
                self.pool.insert(
                    pos,
                    Candidate {
                        dist: d,
                        id: u,
                        done: false,
                    },
                );
                self.pool.truncate(ef);
                best = best.min(pos);
            }
            cur = best.min(cur);
            while cur < self.pool.len() && self.pool[cur].done {
                cur += 1;
            }
        }
        out.clear();
        out.extend(self.results.iter().map(|r| r.1));
        exact
    }
}

pub(super) fn push_top(top: &mut Vec<(f32, u32)>, k: usize, dist: f32, id: u32) {
    if top.len() >= k && top.last().is_none_or(|l| dist >= l.0) {
        return;
    }
    let pos = top.partition_point(|c| c.0 <= dist);
    top.insert(pos, (dist, id));
    top.truncate(k);
}
