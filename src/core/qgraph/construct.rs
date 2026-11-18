// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) fn validate_build(
    data: &[f32],
    dim: usize,
    params: &BuildParams,
) -> Result<(), WarmError> {
    if dim == 0 || !data.len().is_multiple_of(dim) {
        return Err(invalid(format!(
            "ragged data: {} values are not a multiple of dimension {dim}",
            data.len()
        )));
    }
    let n = data.len() / dim;
    if n == 0 {
        return Err(invalid("empty index: no vectors to build from".into()));
    }
    if n >= NONE as usize {
        return Err(invalid(format!(
            "index too large: {n} vectors, the limit is {}",
            NONE - 1
        )));
    }
    if let Some(at) = data.iter().position(|v| !v.is_finite()) {
        return Err(invalid(format!(
            "non-finite value in row {} (coordinate {})",
            at / dim,
            at % dim
        )));
    }
    if !(1..=1_048_576).contains(&params.build_ef) {
        return Err(invalid("build_ef must be in 1..=1048576".into()));
    }
    let r = params.degree;
    if r == 0 || r > 65_536 || !r.is_multiple_of(2) {
        return Err(invalid(format!(
            "degree must be positive, even, and at most 65536, not {r}"
        )));
    }
    if !params.alpha.is_finite() || params.alpha < 1.0 {
        return Err(invalid("alpha must be finite and at least 1".into()));
    }
    Ok(())
}

pub(super) fn build_structure(
    vectors: &[f32],
    norms: &[f32],
    dim: usize,
    params: &BuildParams,
) -> Result<GraphStructure, WarmError> {
    let tsetup = prof::start();
    let n = vectors.len() / dim;
    let r = params.degree;
    let metric = params.metric;
    if let Some(at) = norms.iter().position(|v| !v.is_finite()) {
        return Err(invalid(format!("row {at} overflows its squared norm")));
    }
    let codes = (params.quantized_search || dim >= BUILD_CODES_MIN_DIM)
        .then(|| BuildCodes::new(vectors, dim, norms_opt(norms)));
    let rows = Rows {
        data: vectors,
        dim,
        norms: norms_opt(norms),
        codes: codes.as_ref(),
        quantized: params.quantized_search,
        bound: params.profile.bound(),
    };

    let entry = medoid(&rows, n, metric);

    let mut rng = SplitMix(params.seed ^ 0xA5A5_A5A5);
    let mut order: Vec<u32> = (0..n as u32).collect();
    for i in (1..n).rev() {
        order.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }
    prof::report("setup", tsetup);
    // The upper layers depend on the rows and `order` alone; built before the adjacency
    // exists, their gathered rows do not add to its footprint.
    let tup = prof::start();
    let layers = upper_layers(&rows, &order, params);
    prof::report("upper", tup);
    let tloc = prof::start();
    let rank = rows.locality_rank(&order);
    prof::report("locality", tloc);
    let mut graph = Adjacency::new(n, r);
    for (l, alpha) in params.profile.passes(params) {
        let t = prof::start();
        rows.insert_pass(&mut graph, &order, entry, l, alpha, r, &rank);
        prof::report("pass", t);
    }
    let t = prof::start();
    rows.fill(&mut graph, r);
    prof::report("fill", t);
    prof::graph_hash((&graph, &layers, entry));
    let upper_ids = order[..n / LAYER_RATIO].to_vec();
    Ok(GraphStructure {
        adj: graph,
        entry,
        upper_ids,
        layers,
    })
}

/// The row nearest the mean of the `n` rows (angular: of largest dot product with their sum),
/// the entry vertex of a build.
pub(super) fn medoid(rows: &Rows<'_>, n: usize, metric: Metric) -> u32 {
    medoid_by(rows, n, |i| i, metric)
}

pub(super) fn medoid_by(
    rows: &Rows<'_>,
    n: usize,
    row_at: impl Fn(u32) -> u32 + Sync,
    metric: Metric,
) -> u32 {
    let dim = rows.dim;
    let mut centroid = vec![0f32; dim];
    for i in 0..n as u32 {
        let x = rows.row(row_at(i));
        centroid.iter_mut().zip(x).for_each(|(c, v)| *c += v);
    }
    match metric {
        Metric::Angular => {
            (0..n as u32)
                .into_par_iter()
                .map(|i| (dot(rows.row(row_at(i)), &centroid), i))
                .reduce(
                    || (f32::NEG_INFINITY, NONE),
                    |a, b| {
                        if b.0 > a.0 || (b.0 == a.0 && b.1 < a.1) {
                            b
                        } else {
                            a
                        }
                    },
                )
                .1
        }
        Metric::Euclidean => {
            // The row nearest the mean: `|x|^2 - 2 <x, mean>` is `|x - mean|^2` less a
            // constant.
            centroid.iter_mut().for_each(|c| *c /= n as f32);
            (0..n as u32)
                .into_par_iter()
                .map(|i| {
                    (
                        rows.norms.expect("Euclidean rows carry norms")[row_at(i) as usize]
                            - 2.0 * dot(rows.row(row_at(i)), &centroid),
                        i,
                    )
                })
                .reduce(
                    || (f32::INFINITY, NONE),
                    |a, b| {
                        if b.0 < a.0 || (b.0 == a.0 && b.1 < a.1) {
                            b
                        } else {
                            a
                        }
                    },
                )
                .1
        }
    }
}

/// Upper layers (HNSW-like) give each query a nearby entry vertex: layer j holds the first
/// n / 16^(j+1) vertices of `order` (a permutation of the rows), so a vertex keeps its local
/// id in every layer.
pub(super) fn upper_layers(
    rows: &Rows<'_>,
    order: &[u32],
    params: &BuildParams,
) -> Vec<Vec<Vec<u32>>> {
    let dim = rows.dim;
    let mut layers = Vec::new();
    let mut size = order.len() / LAYER_RATIO;
    while size >= MIN_LAYER {
        let sub: Vec<f32> = order[..size]
            .iter()
            .flat_map(|&g| rows.row(g).iter().copied())
            .collect();
        let sub_norms: Vec<f32> = match rows.norms {
            Some(n) => order[..size].iter().map(|&g| n[g as usize]).collect(),
            None => Vec::new(),
        };
        let sub_codes = (params.quantized_search || dim >= BUILD_CODES_MIN_DIM)
            .then(|| BuildCodes::new(&sub, dim, norms_opt(&sub_norms)));
        let sub_rows = Rows {
            data: &sub,
            dim,
            norms: norms_opt(&sub_norms),
            codes: sub_codes.as_ref(),
            quantized: params.quantized_search,
            bound: None,
        };
        let local: Vec<u32> = (0..size as u32).collect();
        let mut g = Adjacency::new(size, UPPER_DEGREE);
        sub_rows.insert_pass(
            &mut g,
            &local,
            0,
            params.build_ef,
            params.alpha,
            UPPER_DEGREE,
            &[],
        );
        layers.push(g.to_lists());
        size /= LAYER_RATIO;
    }
    layers
}

/// FNV-1a over 32-bit words: a stable fingerprint, not a cryptographic hash.
pub(super) struct Fnv(u64);

impl Fnv {
    pub(super) fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(super) fn word(&mut self, w: u32) {
        self.0 = (self.0 ^ u64::from(w)).wrapping_mul(0x0000_0100_0000_01b3);
    }

    pub(super) fn finish(&self) -> u64 {
        self.0
    }
}

/// Bytes of the upper layers and their id map.
pub(super) fn upper_bytes(upper_ids: &[u32], layers: &[Vec<Vec<u32>>]) -> usize {
    let adj: usize = layers.iter().flatten().map(|a| a.len() * 4).sum();
    upper_ids.len() * 4 + adj
}

impl QGraph {
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Bytes held by the index (vertex blocks; the rotation is negligible).
    pub fn index_bytes(&self) -> usize {
        self.blocks.len() * 8 + upper_bytes(&self.upper_ids, &self.layers)
    }

    pub(super) fn vector(&self, v: u32) -> &[f32] {
        &as_f32(&self.block(v)[..self.codes_off])[..self.dim]
    }

    /// Greedy descent through the upper layers with exact distances; returns the bottom-layer
    /// entry vertex and the number of exact evaluations.
    pub(super) fn descend(&self, pq: &[f32]) -> (u32, usize) {
        if self.layers.is_empty() {
            return (self.entry, 0);
        }
        let mut p = 0u32;
        let mut best = -dot(pq, self.vector(self.upper_ids[0]));
        let mut evals = 1;
        for layer in self.layers.iter().rev() {
            loop {
                let mut moved = false;
                for &u in &layer[p as usize] {
                    let d = -dot(pq, self.vector(self.upper_ids[u as usize]));
                    evals += 1;
                    if d < best {
                        best = d;
                        p = u;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        (self.upper_ids[p as usize], evals)
    }

    pub fn searcher(&self) -> Searcher<'_> {
        Searcher {
            index: self,
            visited: Visited::new(self.n),
            pq: vec![0.0; self.rotation.padded()],
            code: QueryCode::new(self.words),
            raw: vec![0; self.degree],
            est: vec![0.0; self.degree],
            pool: Vec::new(),
            results: Vec::new(),
        }
    }

    pub(super) fn block(&self, v: u32) -> &[u64] {
        let v = v as usize;
        &self.blocks[v * self.stride..(v + 1) * self.stride]
    }
}
