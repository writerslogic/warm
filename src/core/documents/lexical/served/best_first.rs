// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl<'a> Run<'a, '_> {
    /// [`Self::negated_top_into`] for one required list ranked without a count: the blocks are
    /// taken strongest bound first, as in [`Self::single_top`], so the k-th best score reaches
    /// its final value early and every other block is skipped on its bound. A visited block
    /// asks the excluded lists about its own slots only. Walking the blocks in slot order
    /// instead raises the floor slowly and decodes many more blocks of a long list.
    #[inline(never)]
    pub(super) fn negated_best_first(
        &mut self,
        lead: &mut Rt<'a>,
        excluded: &mut [Rt<'a>],
        (tables, scatters): (&[Option<std::sync::Arc<Dense>>], &[Option<Scatter>]),
        id: usize,
        lead_bound: Bound,
        top: &mut Top<'a>,
    ) -> Result<()> {
        #[cfg(test)]
        reached(10);
        let idf = lead_bound.idf;
        let cursor = &mut lead.cursors[0];
        let blocks = cursor.block_count();
        let image = self.image;
        let table = image.rank_table(cursor, self.shape.bm25, self.shape.average);
        let mut runs: Vec<(f64, usize)> = Vec::new();
        let order: &[u32] = match table {
            Some(table) => &table.order,
            None => {
                runs.extend(
                    (0..cursor.runs()).map(|run| (cursor.run_weight(run, lead_bound), run)),
                );
                runs.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                &[]
            }
        };
        let run_bound = |cursor: &Cursor<'a>, run: usize| match table {
            Some(table) => RankTable::scale(table.runs[run], idf),
            None => cursor.run_weight(run, lead_bound),
        };
        let mut slots = [0u32; CHECKPOINT_BLOCK];
        let mut rows = [[0u32; CHECKPOINT_BLOCK]; 1];
        let ordered = table.is_some();
        let mut next = 0usize;
        loop {
            let run = if ordered {
                match order.get(next) {
                    Some(&run) => run as usize,
                    None => break,
                }
            } else {
                match runs.get(next) {
                    Some(&(_, run)) => run,
                    None => break,
                }
            };
            next += 1;
            let floor = top.floor();
            if floor.is_some_and(|floor| sum_bound(run_bound(cursor, run), 1) < floor) {
                break;
            }
            for block in run * RUN_BLOCKS..((run + 1) * RUN_BLOCKS).min(blocks) {
                let floor = top.floor();
                cursor.goto_block(block);
                let weight = match table {
                    Some(table) => RankTable::scale(table.blocks[block], idf),
                    None => cursor.block_weight(lead_bound),
                };
                if floor.is_some_and(|floor| sum_bound(weight, 1) < floor) {
                    continue;
                }
                let mut n = cursor.block_slots().len();
                slots[..n].copy_from_slice(cursor.block_slots());
                let mut need = 1u32;
                if let Some(floor) = floor {
                    let (most, least) = cursor.block_bounds();
                    let most = most.min(NEED_PROBES);
                    while need < most && sum_bound(lead_bound.at(need, least), 1) < floor {
                        need += 1;
                    }
                }
                if need > 1 {
                    n = keep_frequent(
                        cursor.block_frequencies(),
                        &mut slots,
                        n,
                        &mut rows[0],
                        need,
                    );
                } else {
                    for (index, value) in rows[0][..n].iter_mut().enumerate() {
                        *value = index as u32;
                    }
                }
                if n == 0 {
                    continue;
                }
                // The excluded cursors only move forward: rewind those past this block.
                let first = slots[0];
                for rt in excluded.iter_mut() {
                    let list = &mut rt.cursors[0];
                    if list.is_end() || list.block_first() > first {
                        list.goto_block(0);
                    }
                }
                n = drop_excluded(excluded, (tables, scatters), &mut slots, n, &mut rows);
                for &slot in &slots[..n] {
                    prefetch(self.word_counts, slot as usize);
                }
                let frequencies = cursor.block_frequencies();
                for (m, &slot) in slots[..n].iter().enumerate() {
                    self.frequencies[id] = frequencies[rows[0][m] as usize];
                    let score = self.score(slot)?;
                    top.offer(score, slot);
                }
            }
        }
        Ok(())
    }
}
