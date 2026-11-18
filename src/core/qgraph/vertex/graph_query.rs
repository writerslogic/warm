// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl VGraph {
    /// Storage geometry and reconstruction error; diagnostic work is never part of search.
    pub fn encoding_diagnostic(&self, source: &Graph) -> Result<serde_json::Value, WarmError> {
        if source.n != self.n || source.dim < self.dim {
            return Err(invalid("encoding diagnostic source shape mismatch".into()));
        }
        let (mut coarse_error, mut fine_error) = (0f64, 0f64);
        for v in 0..self.n {
            let original = self.original(v as u32) as usize;
            for d in 0..self.dim {
                let expected =
                    f64::from(source.vectors[original * source.dim + d]) - f64::from(self.mean[d]);
                let primary = f64::from(self.scales[v])
                    * f64::from(code_at(self.code(v as u32), self.bits, d));
                let residual = if self.has_residual() {
                    f64::from(self.res_scales[v])
                        * f64::from(code_at(
                            &self.residual[v * self.res_stride..(v + 1) * self.res_stride],
                            self.res_bits,
                            d,
                        ))
                } else {
                    0.0
                };
                coarse_error += (primary - expected).powi(2);
                fine_error += (primary + residual - expected).powi(2);
            }
        }
        let count = (self.n * self.dim) as f64;
        Ok(
            serde_json::json!({"metric": format!("{:?}", self.metric), "dimensions": self.dim,
            "padded": self.padded, "primary_stride": self.stride, "residual_stride": self.res_stride,
            "norm_bytes": self.half_norms.len() * 4, "index_bytes": self.index_bytes(),
            "primary_coordinate_rmse": (coarse_error / count).sqrt(),
            "refined_coordinate_rmse": (fine_error / count).sqrt()}),
        )
    }

    /// Runs an untimed traversal control against the graph used to encode this index.
    /// `oracle` is an original row ID; `exact` replaces routing scores only. Residual
    /// reranking, beam ordering and tie handling remain those of the compact index.
    /// Screens, blocks and runtime search knobs are excluded from this control.
    pub fn diagnose(
        &mut self,
        source: &Graph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        oracle: Option<u32>,
        exact: bool,
    ) -> Result<super::super::diagnostics::TraversalDiagnostic, WarmError> {
        if source.n != self.n
            || source.dim != self.dim + usize::from(self.metric == VMetric::InnerProduct)
            || source.degree != self.degree
            || if self.metric == VMetric::InnerProduct {
                source.metric != Metric::Euclidean
            } else {
                self.metric != source.metric.into()
            }
        {
            return Err(invalid(
                "diagnostic source graph does not match index geometry and metric".into(),
            ));
        }
        if self.has_screen() || self.has_blocks() {
            return Err(invalid(
                "diagnostic traversal does not support screens or blocks".into(),
            ));
        }
        if query.len() != self.dim
            || query.iter().any(|x| !x.is_finite())
            || (self.metric == VMetric::Angular && query.iter().all(|&x| x == 0.0))
            || k == 0
            || k > self.n
        {
            return Err(invalid(
                "invalid diagnostic query dimension, coordinates or k".into(),
            ));
        }
        let stored_oracle = match oracle {
            Some(id) if id as usize >= self.n => {
                return Err(invalid("diagnostic oracle ID outside index".into()));
            }
            Some(id) if self.ids.is_empty() => Some(id),
            Some(id) => Some(
                self.ids
                    .iter()
                    .position(|&original| original == id)
                    .expect("encoded IDs are a permutation") as u32,
            ),
            None => None,
        };
        let saved_entry = self.entry;
        let saved_layers = stored_oracle.map(|id| {
            self.entry = id;
            std::mem::take(&mut self.layers)
        });
        let result = if exact {
            self.diagnose_exact(source, query, k, params)
        } else {
            self.diagnose_coded(query, k, params)
        };
        self.entry = saved_entry;
        if let Some(layers) = saved_layers {
            self.layers = layers;
        }
        result
    }

    pub(super) fn diagnostic_entry(
        &self,
        mut score: impl FnMut(u32) -> f32,
        evaluated: &mut Vec<u32>,
    ) -> u32 {
        if self.layers.is_empty() {
            return self.entry;
        }
        let mut p = 0u32;
        let mut best = score(self.upper_ids[0]);
        evaluated.push(self.original(self.upper_ids[0]));
        for layer in self.layers.iter().rev() {
            loop {
                let mut moved = false;
                for &u in &layer[p as usize] {
                    let id = self.upper_ids[u as usize];
                    let candidate = score(id);
                    evaluated.push(self.original(id));
                    if candidate < best {
                        best = candidate;
                        p = u;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        self.upper_ids[p as usize]
    }

    pub(super) fn diagnose_coded(
        &self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
    ) -> Result<super::super::diagnostics::TraversalDiagnostic, WarmError> {
        let mut report = super::super::diagnostics::TraversalDiagnostic::default();
        let mut searcher = self.searcher_with(SearchKnobs::OFF);
        searcher.search(query, k, params, &mut report.results)?;
        let scratch = &mut searcher.scratch;
        let sq = self.encode_query(
            &mut scratch.qf,
            &mut scratch.qc,
            &mut scratch.qtmp,
            &mut scratch.qrot,
            &mut scratch.qs,
        );
        let w = if sq > 0.0 { 1.0 / sq } else { 1.0 };
        // Descent does not set the base-layer marks; replay it after the production search.
        let qsum = query_sum(&scratch.qc);
        self.diagnostic_entry(
            |id| self.est(&scratch.qc, qsum, id, w),
            &mut report.evaluated,
        );
        report.evaluated.extend(
            (0..self.n as u32)
                .filter(|&id| scratch.visited.seen(id))
                .map(|id| self.original(id)),
        );
        report.evaluated.sort_unstable();
        report.evaluated.dedup();
        report
            .beam
            .extend(scratch.pool.keys.iter().map(|&c| self.original(key_id(c))));
        report.retained.extend(
            scratch
                .keys
                .iter()
                .map(|&candidate| self.original(key_id(candidate))),
        );
        if params.rerank > 0 {
            let take = params.rerank.max(k).min(self.n).min(report.retained.len());
            report
                .rerank_candidates
                .extend_from_slice(&report.retained[..take]);
        }
        Ok(report)
    }

    pub(super) fn diagnose_exact(
        &self,
        source: &Graph,
        query: &[f32],
        k: usize,
        params: VSearchParams,
    ) -> Result<super::super::diagnostics::TraversalDiagnostic, WarmError> {
        let mut report = super::super::diagnostics::TraversalDiagnostic::default();
        let score = |id: u32| {
            let original = self.original(id) as usize;
            let row = &source.vectors[original * source.dim..original * source.dim + self.dim];
            let ip: f64 = row
                .iter()
                .zip(query)
                .map(|(&x, &q)| f64::from(x) * f64::from(q))
                .sum();
            let norm = if self.metric == VMetric::Euclidean {
                row.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() * 0.5
            } else {
                0.0
            };
            (norm - ip) as f32
        };
        let mut reservoir = Reservoir::default();
        reservoir.reset(if params.reservoir == 0 {
            0
        } else {
            params.reservoir.max(k).min(self.n)
        });
        let entry = self.diagnostic_entry(
            |id| {
                let distance = score(id);
                reservoir.offer(key(distance, id));
                distance
            },
            &mut report.evaluated,
        );
        let mut pool = Pool {
            keys: Vec::new(),
            expanded: Vec::new(),
            #[cfg(feature = "experimental-batch-pool")]
            merge_keys: Vec::new(),
            #[cfg(feature = "experimental-batch-pool")]
            merge_expanded: Vec::new(),
            cap: params.ef.max(k).max(1),
            cursor: 0,
        };
        let mut seen = vec![false; self.n];
        seen[entry as usize] = true;
        report.evaluated.push(self.original(entry));
        let entry_key = key(score(entry), entry);
        reservoir.offer(entry_key);
        pool.insert(entry_key);
        let mut stale = 0usize;
        while let Some(candidate) = pool.next() {
            if params.patience > 0 && stale >= params.patience {
                break;
            }
            stale += 1;
            let mut count = 0usize;
            self.for_each_neighbour(key_id(candidate) as usize, |id| {
                count += 1;
                if (params.neighbour_cap > 0 && count > params.neighbour_cap) || seen[id as usize] {
                    return;
                }
                seen[id as usize] = true;
                report.evaluated.push(self.original(id));
                let kk = key(score(id), id);
                reservoir.offer(kk);
                if let Some((pos, _)) = pool.insert(kk) {
                    if params.patience > 0 && pos < k {
                        stale = 0;
                    }
                }
            });
        }
        report.evaluated.sort_unstable();
        report.evaluated.dedup();
        report
            .beam
            .extend(pool.keys.iter().map(|&c| self.original(key_id(c))));
        let keys = if params.reservoir > 0 {
            &reservoir.keys
        } else {
            &pool.keys
        };
        report
            .retained
            .extend(keys.iter().map(|&c| self.original(key_id(c))));
        let take = params.rerank.max(k).min(self.n).min(keys.len());
        if params.rerank > 0 {
            report
                .rerank_candidates
                .extend_from_slice(&report.retained[..take]);
            let mut qf = vec![0.0; self.padded];
            qf[..self.dim].copy_from_slice(query);
            let mut results: Vec<_> = keys[..take]
                .iter()
                .map(|&c| (self.fine(&qf, key_id(c)), self.original(key_id(c))))
                .collect();
            results.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            report
                .results
                .extend(results.iter().take(k).map(|&(_, id)| id));
        } else {
            report
                .results
                .extend_from_slice(&report.retained[..k.min(take)]);
        }
        Ok(report)
    }
}
