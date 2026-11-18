// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Format 02 adds the build parameters and a fingerprint of the vectors to the header, so a
/// cache written for other data or parameters is rejected instead of silently reused; 03 the
/// quantized-search flag; 04 the metric.
pub(super) const CACHE_MAGIC: &[u8; 8] = b"HMSGRF04";

/// Fingerprint of the prepared vectors (normalized for angular) a graph was built over.
pub(super) fn data_fingerprint(vectors: &[f32]) -> u64 {
    let mut h = super::super::Fnv::new();
    vectors.iter().for_each(|x| h.word(x.to_bits()));
    h.finish()
}

pub(super) fn put_u32s(w: &mut impl std::io::Write, xs: &[u32]) -> std::io::Result<()> {
    w.write_all(&(xs.len() as u64).to_le_bytes())?;
    for x in xs {
        w.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

pub(super) fn get_u64(r: &mut impl std::io::Read) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

pub(super) fn get_u32s(r: &mut impl std::io::Read, max: u64) -> std::io::Result<Vec<u32>> {
    let len = get_u64(r)?;
    if len > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "graph cache: list longer than the graph",
        ));
    }
    // Read through `take`, so the buffer grows with the bytes the file holds rather than
    // being allocated at the size its lengths claim.
    let want = len.saturating_mul(4);
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(&mut *r, want), &mut bytes)?;
    if bytes.len() as u64 != want {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "graph cache: list shorter than its lengths",
        ));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect())
}

pub(super) fn put_lists<'a>(
    w: &mut impl std::io::Write,
    lists: impl Iterator<Item = &'a [u32]> + Clone,
) -> std::io::Result<()> {
    let lens: Vec<u32> = lists.clone().map(|l| l.len() as u32).collect();
    put_u32s(w, &lens)?;
    put_u32s(w, &lists.flatten().copied().collect::<Vec<u32>>())
}

pub(super) fn get_lists(r: &mut impl std::io::Read, n: u64) -> std::io::Result<Vec<Vec<u32>>> {
    let lens = get_u32s(r, n)?;
    let flat = get_u32s(r, lens.iter().map(|&l| u64::from(l)).sum())?;
    let mut out = Vec::with_capacity(lens.len());
    let mut at = 0;
    for l in lens {
        out.push(flat[at..at + l as usize].to_vec());
        at += l as usize;
    }
    Ok(out)
}

/// Benchmark support: the graph structure (not the vectors) on disk, so that search
/// experiments on one graph need not rebuild it.
impl Graph {
    /// Writes the header (n, dim, degree, build_ef, alpha, seed, entry, data fingerprint,
    /// quantized_search, metric), the adjacency and the upper layers; the vectors are not
    /// stored.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if self.degree != self.build.degree || self.repaired {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "query-trained graph cannot use an untrained graph cache key; encode and save VGraph instead",
            ));
        }
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        use std::io::Write;
        w.write_all(CACHE_MAGIC)?;
        let header = [
            self.n as u64,
            self.dim as u64,
            self.degree as u64,
            self.build.build_ef as u64,
            u64::from(self.build.alpha.to_bits()),
            self.build.seed,
            u64::from(self.entry),
            data_fingerprint(&self.vectors),
            u64::from(self.build.quantized_search),
            self.metric.code(),
        ];
        for x in header {
            w.write_all(&x.to_le_bytes())?;
        }
        put_lists(&mut w, self.adj.iter())?;
        put_u32s(&mut w, &self.upper_ids)?;
        w.write_all(&(self.layers.len() as u64).to_le_bytes())?;
        for l in &self.layers {
            put_lists(&mut w, l.iter().map(Vec::as_slice))?;
        }
        w.flush()
    }

    /// Reads a graph written by [`Graph::save`] for the same `data` (prepared here exactly
    /// as [`Graph::build`] does) and the same `params`. Any mismatch in the vector count,
    /// dimension, degree, build_ef, alpha, seed, quantized search, metric or the data
    /// fingerprint is an error, so a stale cache never stands in for a build; delete the file
    /// to rebuild.
    pub fn load(
        data: &[f32],
        dim: usize,
        path: &std::path::Path,
        params: &BuildParams,
    ) -> std::io::Result<Self> {
        let bad = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidData, m);
        let mut r = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut magic = [0u8; 8];
        std::io::Read::read_exact(&mut r, &mut magic)?;
        if &magic != CACHE_MAGIC {
            return Err(bad(
                "graph cache: bad magic (an older format or not a cache file); delete it to rebuild"
                    .into(),
            ));
        }
        let mut header = [0u64; 10];
        for x in &mut header {
            *x = get_u64(&mut r)?;
        }
        let [n, d, degree, build_ef, alpha_bits, seed, entry, fingerprint, quantized, metric] =
            header;
        if dim == 0 || !data.len().is_multiple_of(dim) {
            return Err(bad("graph cache: ragged data".into()));
        }
        let mut vectors = data.to_vec();
        let norms = prepare_rows(&mut vectors, dim, params.metric);
        let checks: [(&str, u64, u64); 9] = [
            ("metric", metric, params.metric.code()),
            ("n", n, (data.len() / dim) as u64),
            ("dim", d, dim as u64),
            ("degree", degree, params.degree as u64),
            ("build_ef", build_ef, params.build_ef as u64),
            ("alpha", alpha_bits, u64::from(params.alpha.to_bits())),
            ("seed", seed, params.seed),
            ("data fingerprint", fingerprint, data_fingerprint(&vectors)),
            (
                "quantized_search",
                quantized,
                u64::from(params.quantized_search),
            ),
        ];
        if let Some((what, file, want)) = checks.iter().find(|(_, a, b)| a != b) {
            let show = |x: u64| {
                if *what == "alpha" {
                    f32::from_bits(x as u32).to_string()
                } else {
                    format!("{x:#x}")
                }
            };
            return Err(bad(format!(
                "graph cache: {what} {} in the file, {} requested; delete the file to rebuild",
                show(*file),
                show(*want)
            )));
        }
        let n = n as usize;
        if entry >= n as u64 {
            return Err(bad("graph cache: entry out of range".into()));
        }
        let lens = get_u32s(&mut r, n as u64)?;
        let flat = get_u32s(&mut r, lens.iter().map(|&l| u64::from(l)).sum())?;
        let adj = Adjacency::from_flat(lens, &flat, degree as usize)
            .ok_or_else(|| bad("graph cache: list longer than the degree".into()))?;
        drop(flat);
        let upper_ids = get_u32s(&mut r, n as u64)?;
        let nl = get_u64(&mut r)?;
        if nl > 64 {
            return Err(bad("graph cache: too many layers".into()));
        }
        let layers = (0..nl)
            .map(|_| get_lists(&mut r, n as u64))
            .collect::<std::io::Result<Vec<_>>>()?;
        let ok = adj.len() == n
            && adj.iter().flatten().all(|&u| (u as usize) < n)
            && upper_ids.iter().all(|&u| (u as usize) < n)
            && layers.iter().all(|l| {
                l.len() <= upper_ids.len() && l.iter().flatten().all(|&u| (u as usize) < l.len())
            });
        if !ok {
            return Err(bad("graph cache: id out of range".into()));
        }
        Ok(Self {
            n,
            dim,
            degree: degree as usize,
            metric: params.metric,
            vectors,
            norms,
            adj,
            entry: entry as u32,
            upper_ids,
            layers,
            build: params.clone(),
            repaired: false,
        })
    }
}
