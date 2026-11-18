// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Document order by recursive graph bisection (Dhulipala et al., KDD 2016), the order
//! Lucene's `BPIndexReorderer` and PISA's `recursive_graph_bisection` give an index. Documents
//! that share terms end up close together, so posting lists have smaller gaps, denser blocks and
//! tighter block bounds. The result is a function of the forward index alone: every step is
//! sequential per document or an integer count, so the thread count never changes it.

use std::cell::RefCell;

use rayon::prelude::*;

/// Partitions this small are left in the order they reached.
const MIN_PARTITION: usize = 32;
/// Swap rounds per bisection.
const MAX_ITERATIONS: usize = 20;
/// Partitions at least this large compute their gains in parallel.
const PARALLEL_MIN: usize = 1 << 14;

/// The terms of each document, as `terms[offsets[d]..offsets[d + 1]]`, each row sorted and
/// without repeats; term ids are below `term_count`.
pub(super) struct ForwardIndex {
    pub(super) offsets: Vec<usize>,
    pub(super) terms: Vec<u32>,
    pub(super) term_count: usize,
}

impl ForwardIndex {
    fn row(&self, document: u32) -> &[u32] {
        let document = document as usize;
        &self.terms[self.offsets[document]..self.offsets[document + 1]]
    }

    fn documents(&self) -> usize {
        self.offsets.len() - 1
    }
}

#[derive(Default)]
struct Scratch {
    left: Vec<u32>,
    right: Vec<u32>,
    to_right: Vec<f32>,
    to_left: Vec<f32>,
    stamp: Vec<u32>,
    epoch: u32,
}

impl Scratch {
    fn fit(&mut self, terms: usize) {
        if self.left.len() < terms {
            self.left.resize(terms, 0);
            self.right.resize(terms, 0);
            self.to_right.resize(terms, 0.0);
            self.to_left.resize(terms, 0.0);
            self.stamp.resize(terms, 0);
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// The documents `0..n` of `forward` in bisection order.
pub(super) fn order(forward: &ForwardIndex) -> Vec<u32> {
    let mut documents: Vec<u32> = (0..forward.documents() as u32).collect();
    bisect(forward, &mut documents);
    documents
}

fn bisect(forward: &ForwardIndex, documents: &mut [u32]) {
    if documents.len() <= MIN_PARTITION {
        return;
    }
    if documents.len() >= PARALLEL_MIN {
        // Not the thread's scratch: a parallel split lets this thread run other partitions'
        // splits while it waits, and they take the thread's scratch.
        split(forward, documents, &mut Scratch::default());
    } else {
        SCRATCH.with(|scratch| split(forward, documents, &mut scratch.borrow_mut()));
    }
    let middle = documents.len() / 2;
    let (left, right) = documents.split_at_mut(middle);
    if left.len() >= PARALLEL_MIN / 4 {
        rayon::join(|| bisect(forward, left), || bisect(forward, right));
    } else {
        bisect(forward, left);
        bisect(forward, right);
    }
}

/// Bits a list spends on a term held by `degree` of a side's `size` documents, up to a
/// constant: the log-gap cost of the paper.
fn cost(degree: u32, size: f32) -> f32 {
    let degree = degree as f32;
    degree * (size / (degree + 1.0)).log2()
}

/// Moves documents between the two halves of `documents` while a swap lowers the cost.
fn split(forward: &ForwardIndex, documents: &mut [u32], scratch: &mut Scratch) {
    scratch.fit(forward.term_count);
    scratch.epoch = scratch.epoch.wrapping_add(1);
    if scratch.epoch == 0 {
        scratch.stamp.fill(0);
        scratch.epoch = 1;
    }
    let epoch = scratch.epoch;
    let middle = documents.len() / 2;
    let mut touched = Vec::new();
    for (index, &document) in documents.iter().enumerate() {
        for &term in forward.row(document) {
            let term = term as usize;
            if scratch.stamp[term] != epoch {
                scratch.stamp[term] = epoch;
                scratch.left[term] = 0;
                scratch.right[term] = 0;
                touched.push(term);
            }
            if index < middle {
                scratch.left[term] += 1;
            } else {
                scratch.right[term] += 1;
            }
        }
    }
    let (left_size, right_size) = (middle as f32, (documents.len() - middle) as f32);
    let mut gains: Vec<(f32, u32)> = Vec::with_capacity(documents.len());
    for _ in 0..MAX_ITERATIONS {
        for &term in &touched {
            let (left, right) = (scratch.left[term], scratch.right[term]);
            let now = cost(left, left_size) + cost(right, right_size);
            scratch.to_right[term] = if left > 0 {
                now - cost(left - 1, left_size) - cost(right + 1, right_size)
            } else {
                0.0
            };
            scratch.to_left[term] = if right > 0 {
                now - cost(left + 1, left_size) - cost(right - 1, right_size)
            } else {
                0.0
            };
        }
        let (to_right, to_left) = (&scratch.to_right, &scratch.to_left);
        let gain = |index: usize, document: u32| {
            let moves = if index < middle { to_right } else { to_left };
            let total: f32 = forward
                .row(document)
                .iter()
                .map(|&term| moves[term as usize])
                .sum();
            (total, document)
        };
        gains.clear();
        if documents.len() >= PARALLEL_MIN {
            documents
                .par_iter()
                .enumerate()
                .map(|(index, &document)| gain(index, document))
                .collect_into_vec(&mut gains);
        } else {
            gains.extend(
                documents
                    .iter()
                    .enumerate()
                    .map(|(index, &document)| gain(index, document)),
            );
        }
        let (left_gains, right_gains) = gains.split_at_mut(middle);
        let by_gain = |a: &(f32, u32), b: &(f32, u32)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
        left_gains.sort_unstable_by(by_gain);
        right_gains.sort_unstable_by(by_gain);
        let mut swaps = 0;
        for (from_left, from_right) in left_gains.iter_mut().zip(right_gains.iter_mut()) {
            if from_left.0 + from_right.0 <= 0.0 {
                break;
            }
            for &term in forward.row(from_left.1) {
                scratch.left[term as usize] -= 1;
                scratch.right[term as usize] += 1;
            }
            for &term in forward.row(from_right.1) {
                scratch.right[term as usize] -= 1;
                scratch.left[term as usize] += 1;
            }
            std::mem::swap(&mut from_left.1, &mut from_right.1);
            swaps += 1;
        }
        for (slot, &(_, document)) in documents.iter_mut().zip(gains.iter()) {
            *slot = document;
        }
        if swaps == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(rows: &[&[u32]], term_count: usize) -> ForwardIndex {
        let mut offsets = vec![0];
        let mut terms = Vec::new();
        for row in rows {
            terms.extend_from_slice(row);
            offsets.push(terms.len());
        }
        ForwardIndex {
            offsets,
            terms,
            term_count,
        }
    }

    fn log_gaps(forward: &ForwardIndex, order: &[u32]) -> f64 {
        let mut position = vec![0u32; order.len()];
        for (at, &document) in order.iter().enumerate() {
            position[document as usize] = at as u32;
        }
        let mut lists = vec![Vec::new(); forward.term_count];
        for document in 0..order.len() as u32 {
            for &term in forward.row(document) {
                lists[term as usize].push(position[document as usize]);
            }
        }
        lists
            .iter_mut()
            .map(|list| {
                list.sort_unstable();
                list.windows(2)
                    .map(|pair| f64::from(pair[1] - pair[0]).log2())
                    .sum::<f64>()
            })
            .sum()
    }

    #[test]
    fn interleaved_clusters_come_apart_and_the_order_is_a_permutation() {
        // Two vocabularies, documents alternating between them.
        let rows: Vec<Vec<u32>> = (0..4096u32)
            .map(|d| {
                let base = (d % 2) * 64;
                (0..16).map(|t| base + (d / 2 + t * 7) % 64).collect()
            })
            .map(|mut row: Vec<u32>| {
                row.sort_unstable();
                row.dedup();
                row
            })
            .collect();
        let slices: Vec<&[u32]> = rows.iter().map(Vec::as_slice).collect();
        let forward = forward(&slices, 128);
        let ordered = order(&forward);
        let mut sorted = ordered.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..4096).collect::<Vec<_>>());
        let identity: Vec<u32> = (0..4096).collect();
        assert!(log_gaps(&forward, &ordered) < 0.9 * log_gaps(&forward, &identity));
        assert_eq!(order(&forward), ordered, "the order is deterministic");
    }

    #[test]
    fn tiny_and_termless_inputs_keep_their_order() {
        let empty: [&[u32]; 0] = [];
        assert!(order(&forward(&empty, 0)).is_empty());
        let rows: Vec<&[u32]> = vec![&[]; 100];
        assert_eq!(order(&forward(&rows, 0)), (0..100).collect::<Vec<_>>());
    }
}
