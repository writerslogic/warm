// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[derive(Clone, Copy)]
struct ClassCover {
    members: u64,
    maxima: u64,
}

impl ClassCover {
    fn new(members: u64) -> Self {
        let mut maxima = members;
        for mask in 0..64 {
            if members & (1 << mask) != 0 {
                let mut subset = mask;
                while subset != 0 {
                    subset = (subset - 1) & mask;
                    maxima &= !(1 << subset);
                }
            }
        }
        Self { members, maxima }
    }

    fn upper(self, weights: &[f64]) -> f64 {
        let mut possible = self.maxima;
        let mut upper = 0.0f64;
        while possible != 0 {
            let mut mask = possible.trailing_zeros() as usize;
            possible &= possible - 1;
            let mut sum = 0.0;
            while mask != 0 {
                sum += weights[mask.trailing_zeros() as usize];
                mask &= mask - 1;
            }
            upper = upper.max(sum);
        }
        upper
    }
}

impl<'a> Run<'a, '_> {
    /// Rank with a shared class cover, reusing the seed heap for any disjoint-class fallback.
    #[inline(never)]
    pub(super) fn union_classes_cover(
        &mut self,
        mut terms: Vec<Rt<'a>>,
        reopen: &dyn Fn(usize) -> Option<Rt<'a>>,
    ) -> Result<Option<Vec<LexicalHit>>> {
        // The ranking never counts.
        self.count = false;
        let n = terms.len();
        let ceilings: Vec<f64> = terms.iter().map(|rt| rt.ceiling).collect();
        let clauses: Vec<usize> = terms.iter().map(|rt| rt.clause).collect();
        let mut lengths = [0u64; 6];
        for (length, term) in lengths.iter_mut().zip(&terms) {
            *length = term.rarest() as u64;
        }
        let mut top = Top::new(self.image, self.k);
        merge_twins(&mut terms);
        self.terms_top_into(&mut terms, &mut top)?;
        drop(terms);
        if top.floor().is_none() {
            #[cfg(test)]
            reached(8);
            return Ok(None);
        }
        let mut classes: Vec<(f64, usize)> = (1..(1usize << n) - 1)
            .map(|mask| {
                let sum = (0..n)
                    .filter(|b| mask >> b & 1 == 1)
                    .map(|b| ceilings[b])
                    .sum();
                (sum, mask)
            })
            .collect();
        if self.cover_union_classes(&clauses, &lengths[..n], &classes, reopen, &mut top)? {
            return Ok(Some(top.finish()));
        }
        classes.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (sum, mask) in classes {
            let present: Vec<usize> = (0..n)
                .filter(|b| mask >> b & 1 == 1)
                .map(|b| clauses[b])
                .collect();
            let absent: Vec<usize> = (0..n)
                .filter(|b| mask >> b & 1 == 0)
                .map(|b| clauses[b])
                .collect();
            if top
                .floor()
                .is_some_and(|floor| sum_bound(sum, present.len()) < floor)
            {
                #[cfg(test)]
                reached(9);
                continue;
            }
            // The previous class left its frequencies in place.
            self.frequencies.fill(0);
            #[cfg(test)]
            reached(5 + present.len());
            let mut musts = self.opened(reopen, &present);
            let mut excluded = self.opened(reopen, &absent);
            merge_twins(&mut musts);
            self.negated_top_into(&mut musts, &mut excluded, &mut top, true)?;
        }
        Ok(Some(top.finish()))
    }

    /// Each competitive class belongs to the first inclusion-minimal conjunction it matches.
    /// A lead block uses the maximum sum over only the optional patterns assigned to its scan.
    #[inline(never)]
    fn cover_union_classes(
        &mut self,
        clauses: &[usize],
        lengths: &[u64],
        classes: &[(f64, usize)],
        reopen: &dyn Fn(usize) -> Option<Rt<'a>>,
        top: &mut Top<'a>,
    ) -> Result<bool> {
        let floor = top.floor().expect("a seeded heap");
        let lead_size = |mask: usize| {
            lengths
                .iter()
                .enumerate()
                .filter(|&(i, _)| mask & (1 << i) != 0)
                .map(|(_, &len)| len)
                .min()
                .expect("a nonempty class")
        };
        let mut live = 0u64;
        let mut separate = 0u64;
        for &(sum, mask) in classes {
            if sum_bound(sum, mask.count_ones() as usize) >= floor {
                live |= 1 << mask;
                separate += lead_size(mask);
            }
        }
        let mut minima: Vec<_> = classes
            .iter()
            .filter_map(|&(_, mask)| {
                if live & (1 << mask) == 0 {
                    return None;
                }
                let mut bits = mask;
                while bits != 0 {
                    let subset = mask ^ (1 << bits.trailing_zeros());
                    if live & (1 << subset) != 0 {
                        return None;
                    }
                    bits &= bits - 1;
                }
                Some((lead_size(mask), mask))
            })
            .collect();
        let merged: u64 = minima.iter().map(|&(len, _)| len).sum();
        if separate == 0 || merged * 5 > separate * 4 {
            return Ok(false);
        }
        minima.sort_unstable();
        self.frequencies.fill(0);
        #[cfg(test)]
        CLASS_COVERS.with(|calls| calls.set(calls.get() + 1));
        for (_, minimum) in minima {
            let present: Vec<_> = (0..clauses.len())
                .filter(|&i| minimum & (1 << i) != 0)
                .map(|i| clauses[i])
                .collect();
            let rest: Vec<_> = (0..clauses.len())
                .filter(|&i| minimum & (1 << i) == 0)
                .collect();
            let mut members = 0u64;
            for (bound, mask) in classes.iter().copied() {
                if mask & minimum != minimum || live & (1 << mask) == 0 {
                    continue;
                }
                live &= !(1 << mask);
                if sum_bound(bound, mask.count_ones() as usize)
                    < top.floor().expect("a seeded heap")
                {
                    continue;
                }
                let mut extra = 0;
                for (j, &i) in rest.iter().enumerate() {
                    extra |= ((mask >> i) & 1) << j;
                }
                members |= 1 << extra;
            }
            if members == 0 {
                continue;
            }
            let mut musts = self.opened(reopen, &present);
            merge_twins(&mut musts);
            let filters = musts.split_off(1);
            let lead = musts.pop().expect("a required term");
            let others: Vec<_> = rest.iter().map(|&i| clauses[i]).collect();
            self.lead_top_cover(
                lead,
                filters,
                ClassCover::new(members),
                self.opened(reopen, &others),
                top,
            )?;
        }
        debug_assert_eq!(live, 0, "every competitive class has a cover");
        Ok(true)
    }

    /// Ranks a lead's blocks using bounds for the optional patterns assigned to this cover.
    #[inline(never)]
    fn lead_top_cover(
        &mut self,
        mut lead: Rt<'a>,
        filters: Vec<Rt<'a>>,
        cover: ClassCover,
        optional: Vec<Rt<'a>>,
        top: &mut Top<'a>,
    ) -> Result<()> {
        // The other required lists, which filter the candidates, then the optional ones.
        let filtering = filters.len();
        // Built without `chain().collect()`, whose instance `lead_top` shares and would then
        // call out of line instead of inlining.
        let mut optional = {
            let mut all = Vec::with_capacity(filtering + optional.len());
            all.extend(filters);
            all.extend(optional);
            all
        };
        let tables: Vec<_> = optional
            .iter()
            .map(|rt| self.image.frequency_table(rt.cursors[0].list()))
            .collect();
        let clauses = 1 + optional.len();
        let lead_bound = lead.bound.expect("a bounded term");
        let ceiling = sum_bound(
            lead.ceiling + optional.iter().map(|rt| rt.ceiling).sum::<f64>(),
            clauses,
        );
        let mut slots = [0u32; CHECKPOINT_BLOCK];
        let mut index = [0u32; CHECKPOINT_BLOCK];
        let mut rows = vec![[0u32; CHECKPOINT_BLOCK]; optional.len()];
        'blocks: loop {
            let floor = top.floor();
            if floor.is_some_and(|floor| ceiling < floor) {
                break;
            }
            let cursor = &mut lead.cursors[0];
            if cursor.is_end() {
                break;
            }
            let (first, last) = (cursor.block_first(), cursor.block_last());
            let mut rest_sum = 0.0f64;
            if let Some(floor) = floor {
                let lead_weight = cursor.block_weight(lead_bound);
                let mut extras = [0.0f64; 5];
                for (o, other) in optional.iter_mut().enumerate() {
                    let bound = other.bound.expect("a bounded term");
                    let c = &mut other.cursors[0];
                    let live = c.advance_block(first);
                    let in_range = live && c.block_first() <= last;
                    if o < filtering && !in_range {
                        if !live || !lead.cursors[0].advance_block(last.saturating_add(1)) {
                            break 'blocks;
                        }
                        continue 'blocks;
                    }
                    let weight = if !in_range {
                        0.0
                    } else if c.block_last() >= last {
                        c.block_weight(bound)
                    } else {
                        c.span_weight(last, bound, 8, true).unwrap_or(other.ceiling)
                    };
                    if o >= filtering {
                        extras[o - filtering] = weight;
                    } else {
                        rest_sum += weight;
                    }
                }
                rest_sum += cover.upper(&extras);
                if sum_bound(lead_weight + rest_sum, clauses) < floor {
                    if !lead.cursors[0].advance_block(last.saturating_add(1)) {
                        break;
                    }
                    continue;
                }
            }
            let block = lead.cursors[0].block_slots();
            let mut n = block.len();
            slots[..n].copy_from_slice(block);
            // A candidate whose lead frequency, at the block's least word count, cannot lift the
            // block's weights to the k-th best score is dropped before the other lists are read.
            let mut need = 1u32;
            if let Some(floor) = floor {
                let (most, least) = lead.cursors[0].block_bounds();
                let most = most.min(NEED_PROBES);
                while need < most
                    && sum_bound(lead_bound.at(need, least) + rest_sum, clauses) < floor
                {
                    need += 1;
                }
            }
            if need > 1 {
                n = keep_frequent(
                    lead.cursors[0].block_frequencies(),
                    &mut slots,
                    n,
                    &mut index,
                    need,
                );
            } else {
                for (i, value) in index[..n].iter_mut().enumerate() {
                    *value = i as u32;
                }
            }
            for f in 0..filtering {
                if n == 0 {
                    break;
                }
                match &tables[f] {
                    Some(table) => {
                        probe_tabled(table, &mut optional[f].cursors[0], &slots, n, &mut rows[f]);
                    }
                    None => probe_common(&mut optional[f].cursors[0], &slots, n, &mut rows[f]),
                }
                let mut kept = 0usize;
                for m in 0..n {
                    slots[kept] = slots[m];
                    index[kept] = index[m];
                    for row in &mut rows[..=f] {
                        row[kept] = row[m];
                    }
                    kept += usize::from(rows[f][m] > 0);
                }
                n = kept;
            }
            if n > 0 {
                for (o, (other, row)) in optional
                    .iter_mut()
                    .zip(rows.iter_mut())
                    .enumerate()
                    .skip(filtering)
                {
                    match &tables[o] {
                        Some(table) => probe_tabled(table, &mut other.cursors[0], &slots, n, row),
                        None => probe_common(&mut other.cursors[0], &slots, n, row),
                    }
                }
                for &slot in &slots[..n] {
                    prefetch(self.word_counts, slot as usize);
                }
                let frequencies = lead.cursors[0].block_frequencies();
                for (m, &slot) in slots[..n].iter().enumerate() {
                    let mut mask = 0;
                    for (term, row) in rows[filtering..].iter().enumerate() {
                        mask |= usize::from(row[m] != 0) << term;
                    }
                    if cover.members & (1 << mask) == 0 {
                        continue;
                    }
                    self.frequencies[lead.clause] = frequencies[index[m] as usize];
                    for (other, row) in optional.iter().zip(&rows) {
                        self.frequencies[other.clause] = row[m];
                    }
                    let score = self.score(slot)?;
                    top.offer(score, slot);
                }
            }
            if !lead.cursors[0].advance_block(last.saturating_add(1)) {
                break;
            }
        }
        Ok(())
    }
}
