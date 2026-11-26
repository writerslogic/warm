// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl VGraph {
    /// Encodes a built graph with the 8-bit residual code used by `rerank` (or none), unaligned
    /// rows and the build's vertex order.
    pub fn from_graph(g: &Graph, residual: bool) -> Result<Self, WarmError> {
        Self::from_graph_with(g, 8, if residual { 8 } else { 0 }, false, false, 4, false)
    }

    /// Encodes a built graph. `bits` (8 or 4) sizes the traversal code and `residual_bits`
    /// (0, 4 or 8) the re-rank code of its residual; `align` keeps every code row inside as
    /// few 128-byte lines as its size allows; `reorder` renumbers the vertices in breadth-first
    /// order (search still returns the original ids); `id_bytes` 3 stores neighbour ids in
    /// three bytes (graphs below 2^24 - 1 vertices) instead of 4, and 0 stores each list
    /// sorted and delta-coded as varints (sizes vary; `reorder` makes the deltas small); `screen` adds a 1-bit
    /// sign code of the rotated centred vector per vertex (RaBitQ-style, with a per-vertex
    /// factor) that the search consults before paying for the 8-bit estimate.
    pub fn from_graph_with(
        g: &Graph,
        bits: u8,
        residual_bits: u8,
        align: bool,
        reorder: bool,
        id_bytes: u8,
        screen: bool,
    ) -> Result<Self, WarmError> {
        Self::encode_graph(
            g,
            (g.dim, g.metric.into()),
            (bits, residual_bits),
            align,
            reorder,
            id_bytes,
            screen,
        )
    }

    /// Keeps a lifted Euclidean graph's topology while encoding only its original coordinates
    /// for negative inner-product scoring. The final construction coordinate is omitted.
    pub fn from_lifted_ip_graph(
        g: &Graph,
        bits: u8,
        residual_bits: u8,
        align: bool,
        reorder: bool,
        id_bytes: u8,
    ) -> Result<Self, WarmError> {
        if g.metric != Metric::Euclidean || g.dim < 2 {
            return Err(invalid(
                "direct IP serving requires a lifted Euclidean graph".into(),
            ));
        }
        Self::encode_graph(
            g,
            (g.dim - 1, VMetric::InnerProduct),
            (bits, residual_bits),
            align,
            reorder,
            id_bytes,
            false,
        )
    }

    pub(super) fn encode_graph(
        g: &Graph,
        geometry: (usize, VMetric),
        precision: (u8, u8),
        align: bool,
        reorder: bool,
        id_bytes: u8,
        screen: bool,
    ) -> Result<Self, WarmError> {
        let (dim, metric) = geometry;
        let (bits, residual_bits) = precision;
        let layout = VertexLayout::new(g.n, dim, bits, residual_bits, align, id_bytes)?;
        let euclidean = metric == VMetric::Euclidean;
        if metric != VMetric::Angular && screen {
            return Err(invalid("the 1-bit screen is angular only".into()));
        }
        let n = g.n;
        let VertexLayout {
            padded,
            cbytes,
            stride,
            res_stride,
        } = layout;
        let mut sum = vec![0f64; dim];
        for x in g.vectors.chunks_exact(g.dim) {
            sum.iter_mut().zip(x).for_each(|(s, &v)| *s += f64::from(v));
        }
        let mut mean: Vec<f32> = sum.iter().map(|s| (s / n as f64) as f32).collect();
        mean.resize(padded, 0.0);

        let ids = if reorder {
            bfs_order(g.view())
        } else {
            Vec::new()
        };
        let old = |v: usize| if reorder { ids[v] as usize } else { v };
        let mut new_id: Vec<u32> = Vec::new();
        if reorder {
            new_id = vec![0; n];
            for (v, &o) in ids.iter().enumerate() {
                new_id[o as usize] = v as u32;
            }
        }
        let new = |o: u32| if reorder { new_id[o as usize] } else { o };
        let half_norms: Vec<f32> = if euclidean {
            let mut half_norms = Vec::with_capacity(n);
            huge_capacity(half_norms.as_ptr(), n);
            half_norms.extend((0..n).map(|v| 0.5 * g.norms[old(v)]));
            half_norms
        } else {
            Vec::new()
        };

        let screen_rot = screen.then(|| Rotation::new(dim, 0, SCREEN_SEED));
        let screen_words = screen_rot.as_ref().map_or(0, |r| r.padded() / 64);
        // The centred row of stored vertex `v`, which every code below encodes, written to a
        // per-worker buffer.
        let centred = |v: usize, out: &mut Vec<f32>| {
            let o = old(v);
            out.clear();
            out.extend(
                g.vectors[o * g.dim..o * g.dim + dim]
                    .iter()
                    .zip(&mean)
                    .map(|(a, m)| a - m),
            );
        };
        // Codes are written in place, so no per-vertex buffers sit next to the final arrays.
        let mut codes = Lines::zeroed(n * stride);
        let mut scales = Vec::with_capacity(n);
        huge_capacity(scales.as_ptr(), n);
        scales.resize(n, 0f32);
        codes
            .bytes_mut()
            .par_chunks_exact_mut(stride)
            .zip(scales.par_iter_mut())
            .enumerate()
            .for_each_init(Vec::<f32>::new, |x, (v, (dst, s))| {
                centred(v, x);
                *s = if bits == 8 {
                    quantize(x, &mut dst[..cbytes])
                } else {
                    quantize4(x, &mut dst[..cbytes])
                };
            });
        let mut residual = Vec::with_capacity(n * res_stride);
        huge_capacity(residual.as_ptr(), n * res_stride);
        residual.resize(n * res_stride, 0i8);
        let mut res_scales = vec![0f32; if residual_bits > 0 { n } else { 0 }];
        if residual_bits > 0 {
            let code_bytes = codes.bytes();
            residual
                .par_chunks_exact_mut(res_stride)
                .zip(res_scales.par_iter_mut())
                .enumerate()
                .for_each_init(Vec::<f32>::new, |x, (v, (r, sr))| {
                    centred(v, x);
                    let c = &code_bytes[v * stride..v * stride + cbytes];
                    let s = scales[v];
                    x.iter_mut()
                        .enumerate()
                        .for_each(|(i, a)| *a -= s * f32::from(code_at(c, bits, i)));
                    *sr = if residual_bits == 8 {
                        quantize(x, r)
                    } else {
                        quantize4(x, r)
                    };
                });
        }
        let mut screen_rows = vec![0u64; if screen { n * (screen_words + 1) } else { 0 }];
        if let Some(rot) = &screen_rot {
            screen_rows
                .par_chunks_exact_mut(screen_words + 1)
                .enumerate()
                .for_each_init(
                    || (Vec::<f32>::new(), vec![0f32; rot.padded()]),
                    |(x, xr), (v, srow)| {
                        centred(v, x);
                        rot.apply(x, xr);
                        let (mut norm2, mut abs) = (0f32, 0f32);
                        let mut pop = 0u32;
                        for (i, &a) in xr.iter().enumerate() {
                            norm2 += a * a;
                            abs += a.abs();
                            if a >= 0.0 {
                                srow[i / 64] |= 1u64 << (i % 64);
                                pop += 1;
                            }
                        }
                        // x_r ~= f * sign(x_r) with f = |x_r|^2 / sum |x_r,i| (the RaBitQ factor).
                        let f = if abs > 0.0 { norm2 / abs } else { 0.0 };
                        srow[screen_words] = u64::from(f.to_bits()) | (u64::from(pop) << 32);
                    },
                );
        }

        let r = g.degree;
        // Stored vertex `v`'s list: the build's list of the vertex, renumbered, at most `r`.
        let list = |v: usize| g.neighbors(old(v)).iter().take(r).map(|&u| new(u));
        let (mut adj, mut adj3) = (Vec::new(), Vec::new());
        let (mut adjv, mut adjv_off) = (Vec::new(), Vec::new());
        if id_bytes == 3 {
            adj3 = Vec::<u8>::with_capacity(3 * n * r);
            huge_capacity(adj3.as_ptr(), adj3.capacity());
            for v in 0..n {
                let mut count = 0;
                for u in list(v) {
                    adj3.extend_from_slice(&u.min(NONE3).to_le_bytes()[..3]);
                    count += 1;
                }
                for _ in count..r {
                    adj3.extend_from_slice(&NONE3.to_le_bytes()[..3]);
                }
            }
        } else if id_bytes == 4 {
            adj = vec![NONE; n * r];
            for (v, dst) in adj.chunks_exact_mut(r).enumerate() {
                for (d, u) in dst.iter_mut().zip(list(v)) {
                    *d = u;
                }
            }
        } else {
            adjv_off.reserve(n + 1);
            let mut sorted: Vec<u32> = Vec::with_capacity(r);
            for v in 0..n {
                adjv_off.push(adjv.len() as u32);
                sorted.clear();
                sorted.extend(list(v).take_while(|&u| u != NONE));
                sorted.sort_unstable();
                let list = &sorted;
                let mut prev = v as u32;
                for (i, &u) in list.iter().enumerate() {
                    let delta = if i == 0 {
                        zigzag(i64::from(u) - i64::from(prev))
                    } else {
                        u64::from(u - prev - 1)
                    };
                    put_varint(&mut adjv, delta);
                    prev = u;
                }
            }
            adjv_off.push(adjv.len() as u32);
        }
        let entry = new(g.entry);
        let upper_ids = g.upper_ids.iter().map(|&u| new(u)).collect();
        let mut out = Self {
            n,
            dim,
            metric,
            padded,
            bits,
            cbytes,
            stride,
            degree: r,
            mean,
            half_norms: half_norms.into(),
            adj: adj.into(),
            adj3: adj3.into(),
            adj_off: Vec::new(),
            adjv: adjv.into(),
            adjv_off: adjv_off.into(),
            codes,
            scales: scales.into(),
            res_bits: residual_bits,
            res_stride,
            residual: residual.into(),
            res_scales: res_scales.into(),
            entry,
            upper_ids,
            layers: g.layers.clone(),
            ids: ids.into(),
            inverse: OnceLock::new(),
            screen: screen_rows.into(),
            screen_words,
            screen_rot,
            screen_sigma: 0.0,
            block_bits: 0,
            block_dim: 0,
            block_m: 0,
            blocks_per_vertex: 0,
            blocks: Lines::zeroed(0),
            block_rot: None,
            block_kappa: 0.0,
            lists: None,
        };
        if screen {
            out.screen_sigma = out.measure_screen_sigma(g);
        }
        Ok(out)
    }

    /// Adds neighbour-block codes: every vertex stores, next to its neighbour list, a
    /// `bits`-per-coordinate code of each neighbour's rotated centred vector in the fast-scan
    /// layout (32 codes per block), so one streamed read and one table scan estimate all
    /// neighbours of an expanded vertex instead of one random row fetch per neighbour
    /// (SymphonyQG's layout; Gou et al., SIGMOD 2025). `bits` is 1, 2 or 4.
    pub fn with_blocks(mut self, g: &Graph, bits: u8) -> Result<Self, WarmError> {
        if !matches!(bits, 1 | 2 | 4) {
            return Err(invalid(format!("block bits must be 1, 2 or 4, not {bits}")));
        }
        if self.metric != VMetric::Angular {
            return Err(invalid("neighbour blocks are angular only".into()));
        }
        let (n, dim, r) = (self.n, self.dim, self.degree);
        let rot = Rotation::new(dim, 0, BLOCK_SEED);
        let bd = rot.padded();
        let m = bd * usize::from(bits) / 4;
        let per_vertex = r.div_ceil(BLOCK);
        let record = block_record_bytes(m);
        let ids = &self.ids;
        let old = |v: usize| if ids.is_empty() { v } else { ids[v] as usize };
        let row = |v: usize| &g.vectors[old(v) * dim..(old(v) + 1) * dim];
        let mut blocks = Lines::zeroed(n * per_vertex * record);
        blocks
            .bytes_mut()
            .par_chunks_exact_mut(per_vertex * record)
            .enumerate()
            .for_each_init(
                || {
                    let list = Vec::<u32>::with_capacity(r);
                    (
                        vec![0f32; bd],
                        vec![0f32; dim],
                        vec![0f32; bd],
                        vec![0u8; BLOCK * m],
                        list,
                    )
                },
                |(xr, diff, rr, nibbles, list), (v, dst)| {
                    // Codes are of the residual `r = x_u - x_v` of each neighbour `u` to the
                    // vertex `v` (as in SymphonyQG), so the code error scales with `|r|`; `e`
                    // folds in the query-independent terms `<x_v, r> - s * <levels, rot(x_v)>`,
                    // which makes the estimate's error `<rot(q - x_v), code error>`, scaled by
                    // the query's distance to `v` as well:
                    // `<q, x_u - mean> = <q, x_v - mean> + s * <levels, rot(q)> + e`.
                    // Every buffer is overwritten before it is read: `apply` writes all of its
                    // output, `diff` is rewritten per neighbour, `nibbles` is cleared per block.
                    let xv = row(v);
                    rot.apply(xv, xr);
                    let mut scales = [0f32; BLOCK];
                    let mut offsets = [0f32; BLOCK];
                    list.clear();
                    self.for_each_neighbour(v, |u| list.push(u));
                    for (b, rec) in dst.chunks_exact_mut(record).enumerate() {
                        nibbles.fill(0);
                        scales.fill(0.0);
                        offsets.fill(0.0);
                        for i in 0..BLOCK {
                            let Some(&u) = list.get(b * BLOCK + i) else {
                                break;
                            };
                            let xu = row(u as usize);
                            for (d, (a, b)) in diff.iter_mut().zip(xu.iter().zip(xv)) {
                                *d = a - b;
                            }
                            rot.apply(diff, rr);
                            let nib = &mut nibbles[i * m..(i + 1) * m];
                            let s = encode_block_code(rr, bits, nib);
                            let dot_lv: f32 =
                                (0..bd).map(|j| block_level(nib, bits, j) * xr[j]).sum();
                            let dot_vr: f32 = xv.iter().zip(diff.iter()).map(|(a, b)| a * b).sum();
                            scales[i] = s;
                            offsets[i] = dot_vr - s * dot_lv;
                        }
                        let rec_u8: &mut [u8] = bytemuck::cast_slice_mut(rec);
                        pack32(nibbles, m, &mut rec_u8[..16 * m]);
                        for (i, (s, e)) in scales.iter().zip(&offsets).enumerate() {
                            let at = 16 * m + 4 * i;
                            rec_u8[at..at + 4].copy_from_slice(&s.to_le_bytes());
                            let at = 16 * m + 4 * BLOCK + 4 * i;
                            rec_u8[at..at + 4].copy_from_slice(&e.to_le_bytes());
                        }
                    }
                },
            );
        self.block_bits = bits;
        self.block_dim = bd;
        self.block_m = m;
        self.blocks_per_vertex = per_vertex;
        self.blocks = blocks;
        self.block_rot = Some(rot);
        self.block_kappa = self.measure_block_kappa(g);
        Ok(self)
    }

    /// Unit row `v` (stored numbering) of the build data.
    pub(super) fn unit_row<'g>(&self, g: &'g Graph, v: usize) -> &'g [f32] {
        let o = if self.ids.is_empty() {
            v
        } else {
            self.ids[v] as usize
        };
        &g.vectors[o * self.dim..(o + 1) * self.dim]
    }

    /// Fits `block_kappa` by least squares on a deterministic sample: the squared error of
    /// the block estimate against the exact centred inner product, regressed on
    /// `s^2 * |q - x_v|^2` (index vectors as queries, random expanded vertices).
    pub(super) fn measure_block_kappa(&self, g: &Graph) -> f32 {
        let (nq, nv) = (32.min(self.n), 64.min(self.n));
        let mut rng = SplitMix(BLOCK_SEED ^ 1);
        let mut s = self.searcher();
        let (mut num, mut den) = (0f64, 0f64);
        let (mut diag_sum2, mut diag_n) = (0f64, 0f64);
        let mut scan = [0u32; BLOCK];
        for _ in 0..nq {
            let qi = (rng.next_u64() % self.n as u64) as usize;
            let q = self.unit_row(g, qi).to_vec();
            s.scratch.qf[..self.dim].copy_from_slice(&q);
            let sq = self.encode_query(
                &mut s.scratch.qf,
                &mut s.scratch.qc,
                &mut s.scratch.qtmp,
                &mut s.scratch.qrot,
                &mut s.scratch.qs,
            );
            if sq == 0.0 {
                continue;
            }
            self.encode_block_query(&s.scratch.qf, sq, &mut s.scratch.bqrot, &mut s.scratch.bq);
            let centred = |x: &[f32]| -> f32 {
                q.iter()
                    .zip(x)
                    .zip(&self.mean)
                    .map(|((a, b), m)| a * (b - m))
                    .sum()
            };
            let mut list = Vec::with_capacity(self.degree);
            for _ in 0..nv {
                let v = (rng.next_u64() % self.n as u64) as usize;
                let xv = self.unit_row(g, v);
                let ip_v = centred(xv);
                let d2: f32 = q.iter().zip(xv).map(|(a, b)| (a - b) * (a - b)).sum();
                list.clear();
                self.for_each_neighbour(v, |u| list.push(u));
                for b in 0..self.blocks_per_vertex {
                    let (codes, scales, offsets) = self.block(v, b);
                    scan32(codes, &s.scratch.bq.lut, self.block_m, &mut scan);
                    for (i, &sum) in scan.iter().enumerate() {
                        let Some(&u) = list.get(b * BLOCK + i) else {
                            break;
                        };
                        let sc = le_f32(scales, i);
                        let est = ip_v
                            + sc * (s.scratch.bq.delta * sum as f32 + s.scratch.bq.min_sum)
                            + le_f32(offsets, i);
                        let exact = centred(self.unit_row(g, u as usize));
                        let err = f64::from(est - exact);
                        num += err * err;
                        den += f64::from(sc * sc * d2);
                        diag_sum2 += f64::from(exact - ip_v).powi(2);
                        diag_n += 1.0;
                    }
                }
            }
        }
        if crate::compatibility::var_os("WARM_BLOCK_DIAG").is_some() && diag_n > 0.0 {
            eprintln!(
                "block diag: bits {} kappa {:.4} rms_err {:.5} rms_residual_ip {:.5} pairs {}",
                self.block_bits,
                num / den.max(1e-30),
                (num / diag_n).sqrt(),
                (diag_sum2 / diag_n).sqrt(),
                diag_n
            );
        }
        if den > 0.0 {
            (num / den) as f32
        } else {
            0.0
        }
    }

    /// True if the index carries neighbour-block codes.
    pub fn has_blocks(&self) -> bool {
        self.block_bits > 0
    }

    /// Bytes of the neighbour-block codes and scales.
    pub fn block_bytes(&self) -> usize {
        self.blocks.len
    }

    /// Block `b` of vertex `v`: its packed codes, its 32 scales and its 32 offsets (both as
    /// little-endian f32 bytes).
    #[inline]
    pub(super) fn block(&self, v: usize, b: usize) -> (&[u8], &[u8], &[u8]) {
        let record = block_record_bytes(self.block_m);
        let at = (v * self.blocks_per_vertex + b) * record;
        let bytes = self.blocks.bytes();
        // SAFETY: `i8` and `u8` have the same size and alignment; the slice stays in bounds.
        let rec =
            unsafe { std::slice::from_raw_parts(bytes.as_ptr().add(at).cast::<u8>(), record) };
        let (codes, rest) = rec.split_at(16 * self.block_m);
        let (scales, offsets) = rest.split_at(4 * BLOCK);
        (codes, scales, offsets)
    }

    #[inline]
    pub(super) fn prefetch_blocks(&self, v: usize) {
        let record = block_record_bytes(self.block_m) * self.blocks_per_vertex;
        let at = v * record;
        prefetch_bytes(&self.blocks.bytes()[at..at + record]);
    }

    /// Builds the block lookup table of a query (`qf` padded, `sq` its 8-bit code scale).
    pub(super) fn encode_block_query(
        &self,
        qf: &[f32],
        sq: f32,
        qrot: &mut [f32],
        bq: &mut BlockQuery,
    ) {
        let rot = self.block_rot.as_ref().expect("blocks present");
        rot.apply(&qf[..self.dim], qrot);
        block_table(qrot, self.block_bits, self.block_m, &mut bq.table);
        bq.quantize(self.block_m);
        bq.inv_sq = if sq > 0.0 { 1.0 / sq } else { 1.0 };
    }

    /// Standard deviation of the screen estimate's error against the 8-bit estimate over
    /// index vectors used as queries (deterministic sample), in estimate units.
    pub(super) fn measure_screen_sigma(&self, g: &Graph) -> f32 {
        let (nq, nv) = (128.min(self.n), 1024.min(self.n));
        let mut rng = SplitMix(SCREEN_SEED ^ 1);
        let mut qc = vec![0i8; self.padded];
        let mut qtmp = vec![0i8; self.padded];
        let mut qf = vec![0f32; self.padded];
        let mut qrot = vec![0f32; self.screen_words * 64];
        let mut qs = QueryCode::new(self.screen_words);
        let (mut sum, mut sum2, mut count) = (0f64, 0f64, 0usize);
        for _ in 0..nq {
            let q = (rng.next_u64() % self.n as u64) as usize;
            qf[..self.dim].copy_from_slice(&g.vectors[q * self.dim..(q + 1) * self.dim]);
            let sq = self.encode_query(&mut qf, &mut qc, &mut qtmp, &mut qrot, &mut qs);
            if sq == 0.0 {
                continue;
            }
            for _ in 0..nv {
                let v = (rng.next_u64() % self.n as u64) as u32;
                let d = f64::from(self.screen_est(&qs, v) - self.est(&qc, query_sum(&qc), v, 1.0));
                sum += d;
                sum2 += d * d;
                count += 1;
            }
        }
        if count == 0 {
            return 0.0;
        }
        let mean = sum / count as f64;
        ((sum2 / count as f64) - mean * mean).max(0.0).sqrt() as f32
    }

    /// Writes the traversal code of `qf` (padded) into `qc` and, with a screen, the rotated
    /// query scaled like the code into `qs`; returns the query's quantization scale.
    pub(super) fn encode_query(
        &self,
        qf: &mut [f32],
        qc: &mut [i8],
        qtmp: &mut [i8],
        qrot: &mut [f32],
        qs: &mut QueryCode,
    ) -> f32 {
        let encode = |qf: &[f32], qc: &mut [i8], qtmp: &mut [i8]| {
            if self.bits == 8 {
                quantize(qf, qc)
            } else {
                let s = quantize(qf, qtmp);
                interleave_for_i4(qtmp, qc);
                s
            }
        };
        let mut sq = encode(qf, qc, qtmp);
        if self.metric == VMetric::Angular
            && (sq < f32::MIN_POSITIVE.sqrt()
                || sq > f32::MAX.sqrt() / (127.0 * self.padded as f32))
        {
            // A positive query scale cancels in cosine ranking, but not in overflowing
            // dot products or subnormal quantization. Ordinary queries retain their bits.
            let largest = qf.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            if largest > 0.0 {
                qf.iter_mut().for_each(|v| *v /= largest);
                sq = encode(qf, qc, qtmp);
            }
        }
        if let Some(rot) = &self.screen_rot {
            rot.apply(&qf[..self.dim], qrot);
            if sq > 0.0 {
                // The 8-bit estimate drops the query scale, so the screen works in the same
                // units: rotated query divided by s_q.
                qrot.iter_mut().for_each(|v| *v /= sq);
            }
            qs.encode(qrot);
        }
        sq
    }

    /// True if the index carries the 1-bit screen.
    pub fn has_screen(&self) -> bool {
        !self.screen.is_empty()
    }

    /// Bytes of the screen rows.
    pub fn screen_bytes(&self) -> usize {
        self.screen.len() * 8
    }

    /// 1-bit estimate in the units of [`VGraph::est`]: `-f * sum_i sign_i * q_i`, with the
    /// signed sum from the query's bit planes as `2 * (lo * pop + delta * raw) - sum(q)`.
    #[inline]
    pub(super) fn screen_est(&self, qs: &QueryCode, v: u32) -> f32 {
        let w = self.screen_words;
        let row = &self.screen[v as usize * (w + 1)..(v as usize + 1) * (w + 1)];
        let raw = raw_one(&row[..w], &qs.planes, w) as f32;
        let meta = row[w];
        let f = f32::from_bits(meta as u32);
        let pop = (meta >> 32) as f32;
        -(f * (2.0 * (qs.lo * pop + qs.delta * raw) - qs.sum))
    }

    #[inline]
    pub(super) fn prefetch_screen(&self, v: u32) {
        let w = self.screen_words;
        prefetch(&self.screen[v as usize * (w + 1)..(v as usize + 1) * (w + 1)]);
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Huge pages behind this index's code rows (an upper bound, see the `/proc/self/smaps`
    /// note on [`huge_page_status`]); `None` where `/proc/self/smaps` is unreadable.
    pub fn code_rows_huge_bytes(&self) -> Option<u64> {
        let rows = self.codes.bytes();
        huge_bytes_behind(rows.as_ptr().cast(), rows.len())
    }

    pub fn has_residual(&self) -> bool {
        self.res_bits > 0
    }

    pub fn code_bits(&self) -> u8 {
        self.bits
    }

    pub fn residual_bits(&self) -> u8 {
        self.res_bits
    }

    /// Logical payload bytes, residual and id map included. See [`VGraphMemoryBreakdown`]
    /// for the accounting boundary and exclusions.
    pub fn index_bytes(&self) -> usize {
        self.memory_breakdown().total_bytes()
    }

    pub fn memory_breakdown(&self) -> VGraphMemoryBreakdown {
        VGraphMemoryBreakdown {
            adjacency_ids_bytes: self.adj.len() * 4 + self.adj3.len() + self.adjv.len(),
            adjacency_offsets_bytes: (self.adj_off.len() + self.adjv_off.len()) * 4,
            traversal_codes_bytes: self.n * self.stride,
            traversal_scales_bytes: self.scales.len() * 4,
            residual_codes_bytes: self.residual.len(),
            residual_scales_bytes: self.res_scales.len() * 4,
            mean_bytes: self.mean.len() * 4,
            norms_bytes: self.half_norms.len() * 4,
            original_ids_bytes: self.ids.len() * 4,
            screen_bytes: self.screen_bytes(),
            neighbor_blocks_bytes: self.block_bytes(),
            upper_layers_bytes: upper_bytes(&self.upper_ids, &self.layers),
        }
    }

    /// Bytes of the residual codes and their scales, which only `rerank` reads.
    pub fn residual_bytes(&self) -> usize {
        self.residual.len() + self.res_scales.len() * 4
    }

    #[inline]
    pub(super) fn code(&self, v: u32) -> &[i8] {
        let at = v as usize * self.stride;
        &self.codes.bytes()[at..at + self.cbytes]
    }

    /// Appends the bottom-layer neighbours of stored vertex `v`, in list order, to `out`.
    pub(in super::super) fn neighbours_into(&self, v: usize, out: &mut Vec<u32>) {
        self.for_each_neighbour(v, |u| out.push(u));
    }

    /// The stored id of the bottom-layer entry vertex.
    pub(in super::super) fn entry_vertex(&self) -> u32 {
        self.entry
    }

    /// Calls `f` on every neighbour of `v`, in list order.
    #[inline]
    pub(super) fn for_each_neighbour(&self, v: usize, mut f: impl FnMut(u32)) {
        if !self.adjv_off.is_empty() {
            return self.for_each_delta_neighbour(v, f);
        }
        let (start, end) = self.adjacency_span(v);
        if self.adj3.is_empty() {
            for &u in &self.adj[start..end] {
                if u == NONE {
                    break;
                }
                f(u);
            }
        } else {
            for b in self.adj3[3 * start..3 * end].as_chunks::<3>().0 {
                let u = u32::from_le_bytes([b[0], b[1], b[2], 0]);
                if u == NONE3 {
                    break;
                }
                f(u);
            }
        }
    }

    #[cfg(feature = "experimental-neighbor-admission")]
    #[inline(always)]
    pub(super) fn admit_neighbour<const SCREEN: bool>(
        &self,
        u: u32,
        visited: &mut Marks,
        fresh: &mut Vec<u32>,
    ) {
        if visited.insert(u) {
            if SCREEN {
                self.prefetch_screen(u);
            } else {
                prefetch_bytes(self.code(u));
                self.prefetch_scale(u);
            }
            fresh.push(u);
        }
    }

    #[cfg(feature = "experimental-neighbor-admission")]
    #[inline(always)]
    pub(super) fn admit_neighbours<const PROFILE: bool, const SCREEN: bool>(
        &self,
        v: usize,
        cap: usize,
        visited: &mut Marks,
        fresh: &mut Vec<u32>,
    ) -> usize {
        let mut neighbors_read = 0;
        if cap == 0 {
            self.for_each_neighbour(v, |u| {
                if PROFILE {
                    neighbors_read += 1;
                }
                self.admit_neighbour::<SCREEN>(u, visited, fresh);
            });
        } else {
            let mut seen = 0usize;
            self.for_each_neighbour(v, |u| {
                seen += 1;
                if seen <= cap {
                    self.admit_neighbour::<SCREEN>(u, visited, fresh);
                }
            });
        }
        neighbors_read
    }

    #[inline]
    pub(super) fn adjacency_span(&self, v: usize) -> (usize, usize) {
        if self.adj_off.is_empty() {
            (v * self.degree, (v + 1) * self.degree)
        } else {
            (self.adj_off[v] as usize, self.adj_off[v + 1] as usize)
        }
    }

    /// Attaches candidate lists built over this index's base rows (row ids as passed to the
    /// build), which a search consults when `probes > 0`. [`VGraph::save`] keeps them.
    pub fn set_query_lists(&mut self, mut lists: QueryLists) -> Result<(), WarmError> {
        if lists.dim() != self.dim {
            return Err(invalid(format!(
                "query lists have {} dimensions, the index is queried in {}",
                lists.dim(),
                self.dim
            )));
        }
        let mut map = vec![0u32; self.n];
        for v in 0..self.n as u32 {
            map[self.original(v) as usize] = v;
        }
        lists.remap(&map)?;
        self.lists = Some(lists);
        self.pack_lists();
        Ok(())
    }

    /// Copies the attached lists' codes into list order when the plain 8-bit row-code scan
    /// can use them (angular or inner-product order); other indexes scan by gather.
    pub(super) fn pack_lists(&mut self) {
        let Some(mut lists) = self.lists.take() else {
            return;
        };
        if self.bits == 8 && self.metric != VMetric::Euclidean {
            lists.pack(self.cbytes, |v| (self.code(v), self.scales[v as usize]));
        }
        self.lists = Some(lists);
    }

    /// The attached candidate lists, if any.
    pub fn query_lists(&self) -> Option<&QueryLists> {
        self.lists.as_ref()
    }

    /// Removes unused adjacency slots without changing neighbors or their order.
    /// Saving expands the lists into the original persisted format; reopening uses that
    /// format until this method is called again. Delta-coded lists already have exact spans.
    pub fn compact_adjacency(&mut self) -> Result<(), WarmError> {
        if !self.adj_off.is_empty() || !self.adjv_off.is_empty() {
            return Ok(());
        }
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(self.n + 1)
            .map_err(|_| invalid("cannot allocate adjacency offsets".into()))?;
        offsets.push(0u32);
        for v in 0..self.n {
            let mut count = 0u32;
            self.for_each_neighbour(v, |_| count += 1);
            let end = offsets[v]
                .checked_add(count)
                .ok_or_else(|| invalid("compact adjacency exceeds u32 offsets".into()))?;
            offsets.push(end);
        }
        let edges = offsets[self.n] as usize;
        let width = if self.adj3.is_empty() { 4 } else { 3 };
        if (self.n * self.degree - edges).saturating_mul(width) <= offsets.len() * 4 {
            return Ok(());
        }
        if self.adj3.is_empty() {
            let mut compact = Vec::new();
            compact
                .try_reserve_exact(edges)
                .map_err(|_| invalid("cannot allocate compact adjacency".into()))?;
            for v in 0..self.n {
                self.for_each_neighbour(v, |u| compact.push(u));
            }
            self.adj = compact.into();
        } else {
            let bytes = edges
                .checked_mul(3)
                .ok_or_else(|| invalid("compact adjacency size overflows".into()))?;
            let mut compact = Vec::new();
            compact
                .try_reserve_exact(bytes)
                .map_err(|_| invalid("cannot allocate compact adjacency".into()))?;
            for v in 0..self.n {
                self.for_each_neighbour(v, |u| compact.extend_from_slice(&u.to_le_bytes()[..3]));
            }
            self.adj3 = compact.into();
        }
        self.adj_off = offsets;
        Ok(())
    }

    /// [`VGraph::for_each_neighbour`] over a delta-coded list.
    #[cold]
    #[inline(never)]
    pub(super) fn for_each_delta_neighbour(&self, v: usize, mut f: impl FnMut(u32)) {
        let span = &self.adjv[self.adjv_off[v] as usize..self.adjv_off[v + 1] as usize];
        let mut at = 0;
        let mut prev = v as u32;
        let mut first = true;
        while at < span.len() {
            let (delta, used) = get_varint(&span[at..]);
            at += used;
            let u = if first {
                (i64::from(prev) + unzigzag(delta)) as u32
            } else {
                prev + delta as u32 + 1
            };
            first = false;
            prev = u;
            f(u);
        }
    }

    #[inline]
    pub(super) fn prefetch_neighbours(&self, v: usize) {
        let (start, end) = self.adjacency_span(v);
        if !self.adjv_off.is_empty() {
            prefetch_u8(&self.adjv[self.adjv_off[v] as usize..self.adjv_off[v + 1] as usize]);
        } else if self.adj3.is_empty() {
            prefetch_ids(&self.adj[start..end]);
        } else {
            prefetch_u8(&self.adj3[3 * start..3 * end]);
        }
    }

    /// Prefetches the per-vertex terms of an estimate of `v`: its scale and, for Euclidean,
    /// its norm term (each its own array, one line per random vertex).
    #[inline]
    pub(super) fn prefetch_scale(&self, v: u32) {
        prefetch_f32(&self.scales[v as usize]);
        if self.metric == VMetric::Euclidean {
            prefetch_f32(&self.half_norms[v as usize]);
        }
    }

    /// Prefetches what [`VGraph::fine`] reads for `v`.
    #[inline]
    pub(super) fn prefetch_rerank_rows(&self, v: u32) {
        prefetch_bytes(self.code(v));
        self.prefetch_scale(v);
        if self.has_residual() {
            let v = v as usize;
            prefetch_bytes(&self.residual[v * self.res_stride..(v + 1) * self.res_stride]);
            prefetch_f32(&self.res_scales[v]);
        }
    }

    #[inline]
    pub(super) fn original(&self, v: u32) -> u32 {
        if self.ids.is_empty() {
            v
        } else {
            self.ids[v as usize]
        }
    }

    /// Estimated negative inner product with the query code `qc`, whose coordinate sum is
    /// `qsum` (up to the per-query terms
    /// `<q, mean>` and `s_q`, which do not change the order). Euclidean adds `w |x|^2 / 2`
    /// with `w = 1 / s_q`, the order of `|q - x|^2` in the same units.
    #[inline]
    pub(super) fn est(&self, qc: &[i8], qsum: i32, v: u32, w: f32) -> f32 {
        let dot = if self.bits == 8 {
            dot_i8_query(qc, self.code(v), qsum) as f32
        } else {
            dot_i4_query(qc, self.code(v), qsum) as f32
        };
        self.order(v, self.scales[v as usize] * dot, w)
    }

    /// The distance order of `v` from its estimated (or re-ranked) inner product `ip` in
    /// units of `1 / w`: `-ip` for angular, `w |x|^2 / 2 - ip` for Euclidean.
    #[inline]
    pub(super) fn order(&self, v: u32, ip: f32, w: f32) -> f32 {
        if self.metric == VMetric::Euclidean {
            self.half_norms[v as usize] * w - ip
        } else {
            -ip
        }
    }

    /// [`VGraph::est`] of four vertices with shared query loads; the same integer sums, so
    /// the same values.
    #[inline]
    pub(super) fn est4<const QUERY_X4: bool>(
        &self,
        qc: &[i8],
        qsum: i32,
        v: [u32; 4],
        w: f32,
    ) -> [f32; 4] {
        let dots = if self.bits == 8 {
            if QUERY_X4 {
                dot_i8_query_x4(qc, v.map(|u| self.code(u)), qsum).map(|dot| dot as f32)
            } else {
                v.map(|u| dot_i8_query(qc, self.code(u), qsum) as f32)
            }
        } else {
            dot_i4_query_x4(qc, v.map(|u| self.code(u)), qsum).map(|dot| dot as f32)
        };
        std::array::from_fn(|i| self.order(v[i], self.scales[v[i] as usize] * dots[i], w))
    }

    /// Re-rank score with the float query: negative estimated inner product with the
    /// reconstructed centred vector (Euclidean: plus `|x|^2 / 2`).
    pub(super) fn fine(&self, qf: &[f32], v: u32) -> f32 {
        if self.bits == 4 && self.res_bits == 8 {
            let row = v as usize;
            let residual = &self.residual[row * self.res_stride..(row + 1) * self.res_stride];
            let (primary, secondary) = dot_f32_i4_i8(qf, self.code(v), residual);
            let ip = self.scales[row] * primary + self.res_scales[row] * secondary;
            return self.order(v, ip, 1.0);
        }
        let primary = if self.bits == 8 {
            dot_f32_i8(qf, self.code(v))
        } else {
            dot_f32_i4(qf, self.code(v))
        };
        let mut ip = self.scales[v as usize] * primary;
        if self.has_residual() {
            let v = v as usize;
            let r = &self.residual[v * self.res_stride..(v + 1) * self.res_stride];
            let d = if self.res_bits == 8 {
                dot_f32_i8(qf, r)
            } else {
                dot_f32_i4(qf, r)
            };
            ip += self.res_scales[v] * d;
        }
        self.order(v, ip, 1.0)
    }

    /// Greedy descent through the upper layers on estimated distances; returns the bottom-layer
    /// entry vertex and the number of estimates.
    pub(super) fn descend(
        &self,
        qc: &[i8],
        qsum: i32,
        w: f32,
        reservoir: &mut Reservoir,
    ) -> (u32, usize) {
        if self.layers.is_empty() {
            return (self.entry, 0);
        }
        let mut p = 0u32;
        let mut best = self.est(qc, qsum, self.upper_ids[0], w);
        reservoir.offer(key(best, self.upper_ids[0]));
        let mut evals = 1;
        for layer in self.layers.iter().rev() {
            loop {
                // The list's rows are fetched together: each step's estimates are independent
                // of one another and only the step's winner decides the next list.
                let list = &layer[p as usize];
                for &u in list {
                    let id = self.upper_ids[u as usize];
                    prefetch_bytes(self.code(id));
                    self.prefetch_scale(id);
                }
                let mut moved = false;
                for &u in list {
                    let id = self.upper_ids[u as usize];
                    let d = self.est(qc, qsum, id, w);
                    reservoir.offer(key(d, id));
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

    /// A searcher with the knobs of the environment ([`SearchKnobs::from_env`]).
    pub fn searcher(&self) -> VSearcher<'_> {
        self.searcher_with(SearchKnobs::from_env())
    }

    /// A searcher with explicit [`SearchKnobs`].
    pub fn searcher_with(&self, knobs: SearchKnobs) -> VSearcher<'_> {
        VSearcher {
            index: self,
            scratch: SearchScratch {
                #[cfg(not(target_arch = "wasm32"))]
                dim: self.dim,
                knobs,
                visited: Marks::new(self.n, self.has_blocks()),
                expanded_hops: None,
                qf: vec![0.0; self.padded],
                qc: vec![0; self.padded],
                qsum: 0,
                qtmp: vec![0; self.padded],
                qrot: vec![0.0; self.screen_words * 64],
                qs: QueryCode::new(self.screen_words.max(1)),
                bqrot: vec![0.0; self.block_dim],
                bq: BlockQuery::new(self.block_m),
                scan: [0; BLOCK],
                fresh: Vec::with_capacity(self.degree),
                survivors: Vec::with_capacity(self.degree),
                screened: 0,
                pool: Pool {
                    keys: Vec::new(),
                    expanded: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_keys: Vec::new(),
                    #[cfg(feature = "experimental-batch-pool")]
                    merge_expanded: Vec::new(),
                    cap: 1,
                    cursor: 0,
                },
                expanded: BinaryHeap::new(),
                reservoir: Reservoir::default(),
                keys: Vec::new(),
                results: Vec::new(),
                probe_query: Vec::new(),
                probed: Vec::new(),
            },
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(in super::super) fn update_encoding(&self) -> super::super::update::Encoding {
        super::super::update::Encoding {
            bits: self.bits,
            residual_bits: self.res_bits,
            align: self.stride != self.cbytes,
            reorder: !self.ids.is_empty(),
            id_bytes: if !self.adjv_off.is_empty() {
                0
            } else if !self.adj3.is_empty() {
                3
            } else {
                4
            },
            screen: self.has_screen(),
            block_bits: self.block_bits,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(in super::super) fn search_reusing(
        &self,
        cache: &mut Option<SearchScratch>,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        self.reusable_scratch(cache)
            .search_validated(self, query, k, params, out)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn reusable_scratch<'a>(
        &self,
        cache: &'a mut Option<SearchScratch>,
    ) -> &'a mut SearchScratch {
        let compatible = cache.as_ref().is_some_and(|s| {
            s.dim == self.dim
                && s.visited.n == self.n
                && s.qf.len() == self.padded
                && s.qrot.len() == self.screen_words * 64
                && s.bqrot.len() == self.block_dim
                && s.bq.table.len() == 16 * self.block_m
                && s.visited.traces.len() == if self.has_blocks() { self.n } else { 0 }
        });
        if !compatible {
            *cache = Some(self.searcher().scratch);
        }
        cache.as_mut().expect("initialized search scratch")
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn search_two_hop_cached(
        &self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        const MAX_CACHED_ROWS: usize = 1 << 20;
        if self.n > MAX_CACHED_ROWS {
            return self.searcher().search_two_hop(query, k, params, out);
        }
        SNAPSHOT_SCRATCH
            .try_with(|slot| {
                let Ok(mut cache) = slot.try_borrow_mut() else {
                    return self.searcher().search_two_hop(query, k, params, out);
                };
                self.reusable_scratch(&mut cache)
                    .search_two_hop(self, query, k, params, out)
            })
            .unwrap_or_else(|_| self.searcher().search_two_hop(query, k, params, out))
    }

    /// `search` on a per-thread cached scratch, so repeated queries on one thread do not
    /// allocate the visited marks each time; indexes over 2^20 rows or with neighbour blocks
    /// use a fresh searcher.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn search_cached(
        &self,
        query: &[f32],
        k: usize,
        params: VSearchParams,
        out: &mut Vec<u32>,
    ) -> Result<usize, WarmError> {
        thread_local! {
            static CACHE: std::cell::RefCell<Option<SearchScratch>> = const { std::cell::RefCell::new(None) };
        }
        if self.n <= 1_048_576 && self.dim <= 4096 && params.ef <= 1024 && !self.has_blocks() {
            SearchScratch::validate_query(self, query)?;
            return CACHE.with(|cache| match cache.try_borrow_mut() {
                Ok(mut cache) => {
                    if let Some(scratch) = cache.as_mut() {
                        scratch.knobs = SearchKnobs::from_env();
                    }
                    self.search_reusing(&mut cache, query, k, params, out)
                }
                Err(_) => self.searcher().search(query, k, params, out),
            });
        }
        self.searcher().search(query, k, params, out)
    }
}
