// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl QGraph {
    /// Build over `data` (row-major, `dim` floats per vector). Rows are normalized; zero rows
    /// stay zero. Uses the rayon pool; the result depends only on the data and `params`.
    pub fn build(data: &[f32], dim: usize, params: &BuildParams) -> Result<Self, WarmError> {
        Self::from_graph(&Graph::build(data, dim, params)?, params)
    }

    /// Encodes a built graph; `params` must be the ones the graph was built with.
    pub fn from_graph(g: &Graph, params: &BuildParams) -> Result<Self, WarmError> {
        let (n, dim, r) = (g.n, g.dim, g.degree);
        if r != params.degree {
            return Err(invalid(format!(
                "graph built with degree {r}, parameters say {}",
                params.degree
            )));
        }
        if g.metric != Metric::Angular {
            return Err(invalid(
                "the edge index is angular only; use VGraph for Euclidean data".into(),
            ));
        }
        let rotation = Rotation::new(dim, params.code_bits, params.seed);
        let words = rotation.padded() / 64;
        let rows = g.rows();
        let graph = &g.adj;
        let codes_off = dim.div_ceil(2);
        let factors_off = codes_off + r * words;
        let ids_off = factors_off + 3 * r / 2;
        let stride = ids_off + r / 2;
        // Every row is rotated once here instead of once per incident edge.
        let padded = rotation.padded();
        let mut rotated = vec![0f32; n * padded];
        rotated
            .par_chunks_mut(padded)
            .enumerate()
            .for_each(|(v, out)| rotation.apply(rows.row(v as u32), out));
        let rot_row = |v: u32| &rotated[v as usize * padded..(v as usize + 1) * padded];
        let mut blocks = vec![0u64; n * stride];
        blocks
            .par_chunks_mut(stride)
            .enumerate()
            .for_each(|(v, b)| {
                let adj = graph.get(v);
                let (vec_w, rest) = b.split_at_mut(codes_off);
                let (codes, rest) = rest.split_at_mut(r * words);
                let (fac, ids) = rest.split_at_mut(3 * r / 2);
                as_f32_mut(vec_w)[..dim].copy_from_slice(rows.row(v as u32));
                let fac = as_f32_mut(fac);
                let ids = as_u32_mut(ids);
                ids.fill(NONE);
                let pv = rot_row(v as u32);
                for &u in adj.iter().take(4) {
                    build_kernels::prefetch_row(rot_row(u));
                }
                for (j, &u) in adj.iter().enumerate() {
                    if let Some(&next) = adj.get(j + 4) {
                        build_kernels::prefetch_row(rot_row(next));
                    }
                    ids[j] = u;
                    let (k, m, pop) =
                        edge_factors(pv, rot_row(u), &mut codes[j * words..(j + 1) * words]);
                    fac[j] = k;
                    fac[r + j] = m;
                    fac[2 * r + j] = pop;
                }
            });
        Ok(Self {
            n,
            dim,
            degree: r,
            words,
            rotation,
            stride,
            codes_off,
            factors_off,
            ids_off,
            blocks,
            entry: g.entry,
            upper_ids: g.upper_ids.clone(),
            layers: g.layers.clone(),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct GraphView<'a> {
    pub(super) n: usize,
    pub(super) dim: usize,
    pub(super) degree: usize,
    pub(super) metric: Metric,
    pub(super) vectors: &'a [f32],
    pub(super) norms: &'a [f32],
    pub(super) adj: &'a Adjacency,
    pub(super) entry: u32,
    pub(super) upper_ids: &'a [u32],
    pub(super) layers: &'a [Vec<Vec<u32>>],
    pub(super) backing: Option<&'a memmap2::Mmap>,
}

impl GraphView<'_> {
    pub(super) fn neighbors(&self, v: usize) -> &[u32] {
        self.adj.get(v)
    }

    pub(super) fn release_rows(&self) -> std::io::Result<()> {
        if let Some(map) = self.backing {
            #[cfg(target_os = "linux")]
            // SAFETY: this shared, read-only mapping owns an unlinked private scratch file;
            // its contents cannot change when discarded pages are read from that file again.
            unsafe {
                map.unchecked_advise(memmap2::UncheckedAdvice::DontNeed)?
            };
            #[cfg(not(target_os = "linux"))]
            let _ = map;
        }
        Ok(())
    }
}

impl Graph {
    pub(super) fn view(&self) -> GraphView<'_> {
        GraphView {
            n: self.n,
            dim: self.dim,
            degree: self.degree,
            metric: self.metric,
            vectors: &self.vectors,
            norms: &self.norms,
            adj: &self.adj,
            entry: self.entry,
            upper_ids: &self.upper_ids,
            layers: &self.layers,
            backing: None,
        }
    }

    pub(super) fn rows(&self) -> Rows<'_> {
        Rows {
            data: &self.vectors,
            dim: self.dim,
            norms: norms_opt(&self.norms),
            codes: None,
            quantized: false,
            bound: None,
        }
    }

    /// The metric the graph was built for.
    pub fn metric(&self) -> Metric {
        self.metric
    }

    /// The out-neighbours of vertex `v` in the base layer.
    pub(crate) fn neighbors(&self, v: usize) -> &[u32] {
        self.adj.get(v)
    }

    /// Stable 64-bit hash of the structure (adjacency, upper layers, entry): equal values mean
    /// a bit-identical graph, across builds, binaries and cache round trips.
    pub fn structure_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.word(self.entry);
        for list in self.adj.iter() {
            h.word(list.len() as u32);
            list.iter().for_each(|&u| h.word(u));
        }
        for layer in &self.layers {
            h.word(layer.len() as u32);
            for list in layer {
                h.word(list.len() as u32);
                list.iter().for_each(|&u| h.word(u));
            }
        }
        h.finish()
    }

    /// Frees the topology (lists, upper layers) once the graph is encoded, keeping the rows
    /// and build parameters that a persisted base still needs. The graph can no longer be
    /// searched, saved or repaired.
    pub fn release_structure(&mut self) {
        self.adj = Adjacency::default();
        self.upper_ids = Vec::new();
        self.layers = Vec::new();
    }

    /// Build over `data` (row-major, `dim` floats per vector). Angular rows are normalized;
    /// zero rows stay zero. Uses the rayon pool; the result depends only on the data and
    /// `params` (`code_bits` is not used).
    pub fn build(data: &[f32], dim: usize, params: &BuildParams) -> Result<Self, WarmError> {
        validate_build(data, dim, params)?;
        Self::build_validated(data.to_vec(), dim, params)
    }

    /// [`Graph::build`] over an owned matrix, prepared in place: the caller's copy is the
    /// one the graph keeps, so the vectors are resident once during construction instead of
    /// twice (the build's own copy next to the caller's).
    pub fn build_owned(
        data: Vec<f32>,
        dim: usize,
        params: &BuildParams,
    ) -> Result<Self, WarmError> {
        validate_build(&data, dim, params)?;
        Self::build_validated(data, dim, params)
    }

    pub(super) fn build_validated(
        mut data: Vec<f32>,
        dim: usize,
        params: &BuildParams,
    ) -> Result<Self, WarmError> {
        let norms = prepare_rows(&mut data, dim, params.metric);
        let structure = build_structure(&data, &norms, dim, params)?;
        Ok(Self {
            n: data.len() / dim,
            dim,
            degree: params.degree,
            metric: params.metric,
            vectors: data,
            norms,
            adj: structure.adj,
            entry: structure.entry,
            upper_ids: structure.upper_ids,
            layers: structure.layers,
            build: params.clone(),
            repaired: false,
        })
    }
}
