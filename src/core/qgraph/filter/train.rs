// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl FilteredIndex {
    /// [`Self::search_batch`] with the queries ordered by their rarest label, so the queries of
    /// one label run back to back on a thread. Results are written back in query order.
    pub(super) fn search_batch_grouped(
        &self,
        queries: &[u8],
        label_indptr: &[u64],
        label_ids: &[u32],
        k: usize,
        params: &FilterSearchParams,
        out: &mut [i64],
    ) -> Result<(), FilterError> {
        let nq = out.len() / k;
        let labels_of =
            |i: usize| &label_ids[label_indptr[i] as usize..label_indptr[i + 1] as usize];
        let mut order: Vec<(u32, u32, u32)> = (0..nq)
            .map(|i| {
                let mut ls = labels_of(i).to_vec();
                ls.sort_by_key(|&l| (self.label_len(l), l));
                ls.dedup();
                let at = |j: usize| ls.get(j).copied().unwrap_or(u32::MAX);
                (at(0), at(1), i as u32)
            })
            .collect();
        order.par_sort_unstable();
        let mut sorted = vec![-1i64; out.len()];
        sorted
            .par_chunks_mut(k)
            .zip(order.par_iter())
            .try_for_each_init(
                || (SearchScratch::default(), Vec::new()),
                |(scratch, found), (row, &(_, _, i))| {
                    let i = i as usize;
                    let q = &queries[i * self.dim..(i + 1) * self.dim];
                    self.search_with_scratch(q, labels_of(i), k, params, found, scratch)?;
                    for (slot, &(_, id)) in row.iter_mut().zip(found.iter()) {
                        *slot = i64::from(id);
                    }
                    Ok(())
                },
            )?;
        for (row, &(_, _, i)) in sorted.chunks(k).zip(&order) {
            out[i as usize * k..(i as usize + 1) * k].copy_from_slice(row);
        }
        Ok(())
    }
}

/// k-means over the members of one label in the shifted i8 domain, then one assignment pass
/// over every member.
pub(super) fn train_label(
    rows: &[i8],
    norms: &[i32],
    dim: usize,
    members: &[u32],
    params: &FilterBuildParams,
    salt: u64,
    coded: Option<(&Pq, &[u8])>,
) -> Result<LabelIvf, FilterError> {
    let lists = members
        .len()
        .div_ceil(params.cluster_size)
        .clamp(1, params.max_lists);
    let row = |id: u32| &rows[id as usize * dim..(id as usize + 1) * dim];
    let train_n = lists
        .saturating_mul(params.train_per_list)
        .min(members.len());
    // Deterministic sample without replacement: a stride walk from a seeded start.
    let mut sample: Vec<u32> = Vec::with_capacity(train_n);
    let start = (mix(params.seed ^ salt) % members.len() as u64) as usize;
    let stride = members.len() as f64 / train_n as f64;
    for i in 0..train_n {
        sample.push(members[(start + (i as f64 * stride) as usize) % members.len()]);
    }
    let elements = product(lists, dim)?;
    let mut centroids = allocation(elements, 0i8)?;
    for c in 0..lists {
        let pick = sample[(c as f64 * train_n as f64 / lists as f64) as usize];
        centroids[c * dim..(c + 1) * dim].copy_from_slice(row(pick));
    }
    let mut cnorms = centroid_norms(&centroids, dim);
    let assign = |x: &[i8], centroids: &[i8], cnorms: &[i32]| -> u32 {
        let qsum: i32 = x.iter().map(|&v| i32::from(v)).sum();
        let mut best = (i64::MAX, 0u32);
        for (c, cent) in centroids.chunks_exact(dim).enumerate() {
            let key = i64::from(cnorms[c]) - 2 * dot_i8_query(x, cent, qsum);
            if key < best.0 {
                best = (key, c as u32);
            }
        }
        best.1
    };
    for _ in 0..params.kmeans_iters {
        let labels: Vec<u32> = sample
            .iter()
            .map(|&id| assign(row(id), &centroids, &cnorms))
            .collect();
        let mut sums = allocation(elements, 0i64)?;
        let mut counts = allocation(lists, 0u32)?;
        for (&id, &c) in sample.iter().zip(&labels) {
            counts[c as usize] += 1;
            let s = &mut sums[c as usize * dim..(c as usize + 1) * dim];
            for (acc, &v) in s.iter_mut().zip(row(id)) {
                *acc += i64::from(v);
            }
        }
        for c in 0..lists {
            if counts[c] == 0 {
                continue;
            }
            let cnt = i64::from(counts[c]);
            for j in 0..dim {
                let s = sums[c * dim + j];
                let m = if s >= 0 {
                    (2 * s + cnt) / (2 * cnt)
                } else {
                    -((-2 * s + cnt) / (2 * cnt))
                };
                centroids[c * dim + j] = m.clamp(-128, 127) as i8;
            }
        }
        cnorms = centroid_norms(&centroids, dim);
    }
    let assignment: Vec<u32> = if members.len() > 50_000 {
        members
            .par_iter()
            .map(|&id| assign(row(id), &centroids, &cnorms))
            .collect()
    } else {
        members
            .iter()
            .map(|&id| assign(row(id), &centroids, &cnorms))
            .collect()
    };
    let mut offsets = vec![0u32; lists + 1];
    for &c in &assignment {
        offsets[c as usize + 1] += 1;
    }
    for c in 0..lists {
        offsets[c + 1] += offsets[c];
    }
    let mut fill = offsets.clone();
    let mut grouped = vec![0u32; members.len()];
    for (&id, &c) in members.iter().zip(&assignment) {
        grouped[fill[c as usize] as usize] = id;
        fill[c as usize] += 1;
    }
    if let Some((pq, point_codes)) = coded {
        let packed = fs::pack_lists(&grouped, &offsets, point_codes, pq.m())?;
        return Ok(LabelIvf {
            centroids,
            centroid_norms: cnorms,
            offsets,
            members: Vec::new(),
            rows: Vec::new(),
            norms: Vec::new(),
            codes: packed.codes,
            bids: packed.ids,
            blocks: packed.blocks,
        });
    }
    let (copy_rows, copy_norms) = if members.len() >= params.copy_min {
        let mut r = allocation(product(grouped.len(), dim)?, 0i8)?;
        for (target, &id) in r.chunks_exact_mut(dim).zip(&grouped) {
            target.copy_from_slice(row(id));
        }
        (r, grouped.iter().map(|&id| norms[id as usize]).collect())
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(LabelIvf {
        centroids,
        centroid_norms: cnorms,
        offsets,
        members: grouped,
        rows: copy_rows,
        norms: copy_norms,
        codes: Vec::new(),
        bids: Vec::new(),
        blocks: Vec::new(),
    })
}

pub(super) fn centroid_norms(centroids: &[i8], dim: usize) -> Vec<i32> {
    centroids
        .chunks_exact(dim)
        .map(|c| c.iter().map(|&v| i32::from(v) * i32::from(v)).sum())
        .collect()
}
