// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PreparedRows {
    pub(super) file: std::io::BufWriter<std::fs::File>,
    pub(super) values: usize,
    pub(super) expected: usize,
    pub(super) dim: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl PreparedRows {
    pub(crate) fn new(count: usize, dim: usize) -> std::io::Result<Self> {
        let invalid =
            || std::io::Error::new(std::io::ErrorKind::InvalidInput, "prepared row geometry");
        if dim == 0 || count >= NONE as usize {
            return Err(invalid());
        }
        let expected = count.checked_mul(dim).ok_or_else(invalid)?;
        expected
            .checked_mul(4)
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or_else(invalid)?;
        Ok(Self {
            file: std::io::BufWriter::with_capacity(1 << 20, tempfile::tempfile()?),
            values: 0,
            expected,
            dim,
        })
    }

    pub(crate) fn append(&mut self, rows: &[f32]) -> std::io::Result<()> {
        use std::io::Write;
        let end = self
            .values
            .checked_add(rows.len())
            .filter(|&n| n <= self.expected);
        if !rows.len().is_multiple_of(self.dim) || end.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "prepared row count",
            ));
        }
        self.file.write_all(bytemuck::cast_slice(rows))?;
        self.values = end.expect("checked row count");
        Ok(())
    }

    pub(crate) fn build(self, params: &BuildParams) -> Result<PreparedGraph, WarmError> {
        let dim = self.dim;
        self.build_with(params, |data, norms| {
            build_structure(data, norms, dim, params)
        })
    }

    /// [`Self::build`] with the graph structure from `structure(rows, norms)` over the
    /// prepared, mapped rows.
    pub(super) fn build_with(
        self,
        params: &BuildParams,
        structure: impl FnOnce(&[f32], &[f32]) -> Result<GraphStructure, WarmError>,
    ) -> Result<PreparedGraph, WarmError> {
        let io = |error: std::io::Error| WarmError::StorageFailure {
            context: format!("prepared graph rows: {error}"),
        };
        if self.values != self.expected || self.values == 0 {
            return Err(invalid("incomplete or empty prepared rows".into()));
        }
        let file = self.file.into_inner().map_err(|e| io(e.into_error()))?;
        // SAFETY: the unlinked temporary file has no other handles or mappings, and remains
        // exclusively owned until this mutable mapping becomes read-only.
        let mut map = unsafe { memmap2::MmapMut::map_mut(&file) }.map_err(io)?;
        let data: &mut [f32] = bytemuck::cast_slice_mut(&mut map[..self.expected * 4]);
        validate_build(data, self.dim, params)?;
        let norms = prepare_rows(data, self.dim, params.metric);
        map.flush().map_err(io)?;
        let vectors = map.make_read_only().map_err(io)?;
        #[cfg(unix)]
        vectors.advise(memmap2::Advice::Random).map_err(io)?;
        let data: &[f32] = bytemuck::cast_slice(&vectors[..self.expected * 4]);
        let structure = structure(data, &norms)?;
        Ok(PreparedGraph {
            vectors,
            norms,
            dim: self.dim,
            metric: params.metric,
            degree: params.degree,
            structure,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PreparedGraph {
    pub(super) vectors: memmap2::Mmap,
    pub(super) norms: Vec<f32>,
    pub(super) dim: usize,
    pub(super) metric: Metric,
    pub(super) degree: usize,
    pub(super) structure: GraphStructure,
}

#[cfg(not(target_arch = "wasm32"))]
impl PreparedGraph {
    pub(super) fn view(&self) -> GraphView<'_> {
        let vectors: &[f32] = bytemuck::cast_slice(&self.vectors[..]);
        GraphView {
            n: vectors.len() / self.dim,
            dim: self.dim,
            degree: self.degree,
            metric: self.metric,
            vectors,
            norms: &self.norms,
            adj: &self.structure.adj,
            entry: self.structure.entry,
            upper_ids: &self.structure.upper_ids,
            layers: &self.structure.layers,
            backing: Some(&self.vectors),
        }
    }
}

/// Prepares rows for `metric` in place: normalizes them for angular (zero rows stay zero) and
/// returns the squared norms for Euclidean (empty for angular).
pub(super) fn prepare_rows(data: &mut [f32], dim: usize, metric: Metric) -> Vec<f32> {
    match metric {
        Metric::Angular => {
            data.par_chunks_mut(dim).for_each(normalize_row);
            Vec::new()
        }
        Metric::Euclidean => data.par_chunks(dim).map(|x| dot(x, x)).collect(),
    }
}

/// Scales `x` to unit length; a zero row stays zero. A finite row whose squared norm
/// overflows to infinity or falls below the normal range is first divided by its largest
/// magnitude, so it normalizes instead of collapsing to zeros or staying unnormalized; other
/// rows take the direct path and keep its exact values.
pub(super) fn normalize_row(x: &mut [f32]) {
    let squared = dot(x, x);
    if squared.is_finite() && squared >= f32::MIN_POSITIVE {
        let norm = squared.sqrt();
        x.iter_mut().for_each(|o| *o /= norm);
        return;
    }
    let largest = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if largest > 0.0 && largest.is_finite() {
        x.iter_mut().for_each(|o| *o /= largest);
        let norm = dot(x, x).sqrt();
        x.iter_mut().for_each(|o| *o /= norm);
    }
}

/// The squared norms as [`Rows`] takes them: `None` when empty (angular).
pub(super) fn norms_opt(norms: &[f32]) -> Option<&[f32]> {
    (!norms.is_empty()).then_some(norms)
}
