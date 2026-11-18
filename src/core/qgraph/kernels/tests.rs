// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// The transform before its first three stages were fused.
fn fwht_reference(x: &mut [f32]) {
    let mut h = 1;
    while h < x.len() {
        for chunk in x.chunks_exact_mut(2 * h) {
            let (a, b) = chunk.split_at_mut(h);
            for (u, v) in a.iter_mut().zip(b.iter_mut()) {
                let t = *u;
                *u = t + *v;
                *v = t - *v;
            }
        }
        h *= 2;
    }
}

#[test]
fn fused_fwht_returns_the_reference_bits_at_every_size() {
    let mut rng = SplitMix(3);
    for log in 0..=12 {
        let n = 1usize << log;
        for _ in 0..4 {
            let x: Vec<f32> = (0..n).map(|_| rng.next_f32() * 2.0 - 1.0).collect();
            let (mut a, mut b) = (x.clone(), x);
            fwht(&mut a);
            fwht_reference(&mut b);
            let bits = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&a), bits(&b), "n {n}");
        }
    }
    // The integer instantiation too.
    let mut a: Vec<i64> = (0..64).map(|i| i * 7 - 200).collect();
    let mut b = a.clone();
    fwht(&mut a);
    let mut h = 1;
    while h < b.len() {
        for chunk in b.chunks_exact_mut(2 * h) {
            let (l, r) = chunk.split_at_mut(h);
            for (u, v) in l.iter_mut().zip(r.iter_mut()) {
                let t = *u;
                *u = t + *v;
                *v = t - *v;
            }
        }
        h *= 2;
    }
    assert_eq!(a, b);
}

/// Development microbenchmark (`--ignored`): ns per transform, fused against reference.
#[test]
#[ignore]
fn microbench_fwht() {
    use std::hint::black_box;
    use std::time::Instant;
    for n in [64usize, 128, 256, 1024] {
        let mut x: Vec<f32> = (0..n).map(|i| (i as f32).sin()).collect();
        let reps = 200_000_000 / (n * n.trailing_zeros() as usize);
        let mut best = [f64::INFINITY; 2];
        for _ in 0..5 {
            for (k, b) in best.iter_mut().enumerate() {
                let t = Instant::now();
                for _ in 0..reps {
                    if k == 0 {
                        fwht(black_box(&mut x));
                    } else {
                        fwht_reference(black_box(&mut x));
                    }
                    x[0] = 0.5;
                }
                *b = b.min(t.elapsed().as_nanos() as f64 / reps as f64);
            }
        }
        println!(
            "{{\"n\": {n}, \"fused_ns\": {:.1}, \"reference_ns\": {:.1}}}",
            best[0], best[1]
        );
    }
}

#[test]
fn popcount_kernels_match_scalar_at_every_width() {
    let seed = 7;
    let mut rng = SplitMix(seed);
    for words in 1..=17 {
        let codes: Vec<u64> = (0..32 * words).map(|_| rng.next_u64()).collect();
        let planes: Vec<u64> = (0..QUERY_BITS * words).map(|_| rng.next_u64()).collect();
        let (mut fast, mut slow) = ([0u32; 32], [0u32; 32]);
        raw_estimates(&codes, &planes, words, &mut fast);
        raw_scalar(&codes, &planes, words, &mut slow);
        assert_eq!(fast, slow, "raw_estimates, words {words}, seed {seed}");
        for (row, (code, want)) in codes.chunks_exact(words).zip(slow).enumerate() {
            assert_eq!(
                raw_one(code, &planes, words),
                want,
                "raw_one, row {row}, words {words}, seed {seed}"
            );
        }
    }
}

/// The POPCNT kernels against the scalar reference, bit for bit, whatever tier is pinned.
#[cfg(target_arch = "x86_64")]
#[test]
fn popcnt_kernels_match_scalar() {
    if !is_x86_feature_detected!("popcnt") {
        return;
    }
    let mut rng = SplitMix(17);
    for words in 0..=17 {
        let rows = 9;
        let codes: Vec<u64> = (0..rows * words).map(|_| rng.next_u64()).collect();
        let planes: Vec<u64> = (0..QUERY_BITS * words).map(|_| rng.next_u64()).collect();
        let mut want = vec![0u32; rows];
        let mut got = vec![u32::MAX; rows];
        if words > 0 {
            raw_scalar(&codes, &planes, words, &mut want);
            // SAFETY: `popcnt` was detected above.
            unsafe { x86::raw_popcnt(&codes, &planes, words, &mut got) };
            assert_eq!(got, want, "words {words}");
        }
        for (row, code) in codes.chunks(words.max(1)).take(rows).enumerate() {
            let code = &code[..words.min(code.len())];
            // SAFETY: `popcnt` was detected above.
            let fast = unsafe { x86::raw_one_popcnt(code, &planes, words) };
            assert_eq!(
                fast,
                raw_one_scalar(code, &planes, words),
                "row {row}, words {words}"
            );
        }
    }
}

/// The dispatched tier is the best one this CPU supports at or below the pin, with the two
/// `vpdpbusd` encodings detected separately: AVX512-VNNI without AVX-VNNI (Cascade Lake)
/// takes the EVEX tier and never the VEX one, and a pin the CPU lacks falls back below it.
#[cfg(target_arch = "x86_64")]
#[test]
fn tier_is_the_best_supported_at_or_below_the_pin() {
    let avx2 = is_x86_feature_detected!("avx2");
    let evex = avx2
        && is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512vl")
        && is_x86_feature_detected!("avx512vnni");
    let vex = avx2 && is_x86_feature_detected!("avxvnni");
    let pin = match crate::compatibility::var("WARM_QGRAPH_KERNEL").as_deref() {
        Ok("scalar") => tier::SCALAR,
        Ok("avx2") => tier::AVX2,
        Ok("avxvnni") => tier::AVXVNNI,
        _ => tier::VNNI,
    };
    let want = if pin >= tier::VNNI && evex {
        tier::VNNI
    } else if pin >= tier::AVXVNNI && vex {
        tier::AVXVNNI
    } else if pin >= tier::AVX2 && avx2 {
        tier::AVX2
    } else {
        tier::SCALAR
    };
    let got = tier::get();
    eprintln!(
        "qgraph kernel tier {got} (1 scalar, 2 avx2, 3 avxvnni, 4 vnni); avx512vnni {evex}, avxvnni {vex}"
    );
    assert_eq!(got, want);
}

/// The four-row i8 entry point on the tier this process runs (`WARM_QGRAPH_KERNEL` pins it)
/// against the scalar sums: every width up to 800 and the extremes of the i8 range.
#[test]
fn dispatched_i8_x4_matches_scalar() {
    let mut rng = SplitMix(29);
    for len in (0..=800).step_by(16) {
        for extreme in [false, true] {
            let mut row = |sign: i8| -> Vec<i8> {
                if extreme {
                    (0..len)
                        .map(|i| if i % 3 == 0 { -128 } else { 127 * sign })
                        .collect()
                } else {
                    random_i8(&mut rng, len)
                }
            };
            let a = row(1);
            let bs = [row(1), row(-1), row(1), row(-1)];
            let four = [&bs[0][..], &bs[1][..], &bs[2][..], &bs[3][..]];
            let want = four.map(|b| i64::from(dot_i8_scalar(&a, b)));
            let qsum = super::super::vnni::query_sum(&a);
            assert_eq!(dot_i8_query_x4(&a, four, qsum), want, "len {len}");
            assert_eq!(four.map(|b| dot_i8_query(&a, b, qsum)), want, "len {len}");
        }
    }
}

#[test]
fn long_i8_dots_widen_before_accumulation_overflows() {
    let limit = i32::MAX as usize / (128 * 128);
    for nonzero in [16, limit - 1, limit, limit + 1, 1 << 20] {
        let len = nonzero.next_multiple_of(16);
        let mut query = vec![-128i8; len];
        query[nonzero..].fill(0);
        let rows: [Vec<i8>; 4] = [-128, 127, -1, 0].map(|value| vec![value; len]);
        let rows = rows.each_ref().map(|row| &row[..]);
        let expected = rows.map(|row| {
            query
                .iter()
                .zip(row)
                .map(|(&q, &x)| i64::from(q) * i64::from(x))
                .sum::<i64>()
        });
        let sum = super::super::vnni::query_sum(&query);
        assert_eq!(rows.map(|row| dot_i8_query(&query, row, sum)), expected);
        assert_eq!(dot_i8_query_x4(&query, rows, sum), expected);
        assert_eq!(dot_i8_query_x4_wide(&query, rows, sum), expected);
    }
}

/// Development microbenchmark (`--ignored`): ns per row of [`dot_i8_query`] and
/// [`dot_i8_query_x4`] at the construction widths, over rows that fit in L2 (4,096) and
/// over a 256 MiB set read at random ids, as construction reads them.
#[test]
#[ignore]
fn microbench_i8_x4() {
    use std::hint::black_box;
    use std::time::Instant;
    let mut rng = SplitMix(5);
    for dim in [128usize, 784] {
        for n in [4096usize, (256 << 20) / dim] {
            let data = random_i8(&mut rng, n * dim);
            let q = random_i8(&mut rng, dim);
            let qs = super::super::vnni::query_sum(&q);
            let ids: Vec<usize> = (0..1 << 16).map(|_| rng.next_u64() as usize % n).collect();
            let row = |i: usize| &data[i * dim..(i + 1) * dim];
            let best = |f: &dyn Fn() -> i64| {
                (0..7)
                    .map(|_| {
                        let t = Instant::now();
                        black_box(f());
                        t.elapsed().as_nanos() as f64 / ids.len() as f64
                    })
                    .fold(f64::INFINITY, f64::min)
            };
            let one = best(&|| {
                ids.iter()
                    .fold(0i64, |a, &i| a.wrapping_add(dot_i8_query(&q, row(i), qs)))
            });
            let four = best(&|| {
                ids.as_chunks::<4>().0.iter().fold(0i64, |a, c| {
                    let r = dot_i8_query_x4(&q, c.map(row), qs);
                    a.wrapping_add(r[0] ^ r[1] ^ r[2] ^ r[3])
                })
            });
            println!(
                "{{\"dim\": {dim}, \"rows\": {n}, \"x1_ns\": {one:.2}, \"x4_ns\": {four:.2}}}"
            );
        }
    }
}

/// The dispatched entry points and the float-by-integer dots, including the empty row;
/// the raw kernels are covered by `every_integer_kernel_matches_scalar`.
#[test]
fn dispatched_and_float_dots_match_scalar() {
    let seed = 13;
    let mut rng = SplitMix(seed);
    for len in (0..=96).step_by(16) {
        let a = random_i8(&mut rng, len);
        let mut b = random_i8(&mut rng, len);
        if let Some(first) = b.first_mut() {
            *first = -128;
        }
        let want = dot_i8_scalar(&a, &b);
        assert_eq!(dot_i8_inline(&a, &b), want, "len {len}, seed {seed}");
        let af: Vec<f32> = a.iter().map(|&x| f32::from(x)).collect();
        assert_eq!(dot_f32_i8(&af, &b), want as f32, "len {len}, seed {seed}");
        let nib: Vec<i8> = b.iter().map(|&x| x >> 4).collect();
        let packed = pack_nibbles(&b);
        let want4 = dot_i8_scalar(&a, &nib);
        assert_eq!(
            dot_f32_i4(&af, &packed),
            want4 as f32,
            "len {len}, seed {seed}"
        );
        if len % 32 == 0 {
            let mut qi = vec![0i8; len];
            interleave_for_i4(&a, &mut qi);
            assert_eq!(dot_i4_scalar(&qi, &packed), want4, "len {len}, seed {seed}");
            assert_eq!(dot_i4_inline(&qi, &packed), want4, "len {len}, seed {seed}");
        }
    }
}

#[test]
fn dot_matches_naive() {
    let mut rng = SplitMix(3);
    for len in 0..70 {
        let a: Vec<f32> = (0..len).map(|_| rng.next_f32() - 0.5).collect();
        let b: Vec<f32> = (0..len).map(|_| rng.next_f32() - 0.5).collect();
        let naive: f64 = a.iter().zip(&b).map(|(x, y)| f64::from(x * y)).sum();
        assert!((f64::from(dot(&a, &b)) - naive).abs() < 1e-5);
    }
}

#[test]
fn rotation_preserves_inner_products() {
    let rot = Rotation::new(100, 0, 11);
    let mut rng = SplitMix(5);
    let a: Vec<f32> = (0..100).map(|_| rng.next_f32() - 0.5).collect();
    let b: Vec<f32> = (0..100).map(|_| rng.next_f32() - 0.5).collect();
    let (mut ra, mut rb) = (vec![0.0; 128], vec![0.0; 128]);
    rot.apply(&a, &mut ra);
    rot.apply(&b, &mut rb);
    assert!((dot(&ra, &rb) - dot(&a, &b)).abs() < 1e-4);
    assert!((dot(&ra, &ra) - dot(&a, &a)).abs() < 1e-4);
}

#[test]
fn query_code_reconstructs_bit_sums() {
    let mut rng = SplitMix(9);
    let x: Vec<f32> = (0..128).map(|_| rng.next_f32() - 0.5).collect();
    let mut qc = QueryCode::new(2);
    qc.encode(&x);
    let code = [rng.next_u64(), rng.next_u64()];
    let mut raw = [0u32; 1];
    raw_estimates(&code, &qc.planes, 2, &mut raw);
    let pop = code.iter().map(|c| c.count_ones()).sum::<u32>() as f32;
    let approx = qc.lo * pop + qc.delta * raw[0] as f32;
    let exact: f32 = (0..128)
        .filter(|i| code[i / 64] >> (i % 64) & 1 == 1)
        .map(|i| x[i])
        .sum();
    // Rounding error is at most delta / 2 per selected coordinate.
    assert!((approx - exact).abs() <= qc.delta / 2.0 * pop + 1e-4);
}

/// Kill test for the 1-bit screen (experiment E2): per neighbour,
/// the screen must cost < 25% of the 8-bit SDOT estimate on a nytimes-sized working set
/// (300k rows, 256 dims: 256-byte code rows, 40-byte screen rows) with the search loop's
/// access pattern (64 random fresh neighbours per expansion, prefetched, then estimated).
/// `cargo test --release raw_screen_microbenchmark -- --ignored --nocapture`; writes JSON
/// to `$WARM_SCREEN_BENCH_OUT` when set.
#[test]
#[ignore = "microbenchmark: run in release with --ignored --nocapture"]
fn raw_screen_microbenchmark() {
    use std::time::Instant;
    const N: usize = 300_000;
    const DIM: usize = 256;
    const WORDS: usize = DIM / 64;
    const DEG: usize = 64;
    const EXPANSIONS: usize = 20_000;
    let mut rng = SplitMix(3);
    let codes: Vec<i8> = (0..N * DIM)
        .map(|_| (rng.next_u64() & 0xff) as i8)
        .collect();
    let screen: Vec<u64> = (0..N * (WORDS + 1)).map(|_| rng.next_u64()).collect();
    let q: Vec<i8> = (0..DIM).map(|_| (rng.next_u64() & 0xff) as i8).collect();
    let x: Vec<f32> = (0..DIM).map(|_| rng.next_f32() - 0.5).collect();
    let mut qc = QueryCode::new(WORDS);
    qc.encode(&x);
    let ids: Vec<u32> = (0..EXPANSIONS * DEG)
        .map(|_| (rng.next_u64() % N as u64) as u32)
        .collect();
    let code = |v: u32| &codes[v as usize * DIM..(v as usize + 1) * DIM];
    let srow = |v: u32| &screen[v as usize * (WORDS + 1)..(v as usize + 1) * (WORDS + 1)];
    let time = |f: &dyn Fn(&[u32]) -> i64| -> (f64, i64) {
        let mut best = f64::INFINITY;
        let mut acc = 0i64;
        for _ in 0..5 {
            let t = Instant::now();
            for batch in ids.as_chunks::<DEG>().0 {
                acc = acc.wrapping_add(f(batch));
            }
            best = best.min(t.elapsed().as_secs_f64() * 1e9 / ids.len() as f64);
        }
        (best, acc)
    };
    let est = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| prefetch_bytes(code(v)));
        batch
            .iter()
            .map(|&v| i64::from(dot_i8_inline(&q, code(v))))
            .sum()
    };
    let scalar = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| prefetch(srow(v)));
        batch
            .iter()
            .map(|&v| i64::from(raw_one_scalar(&srow(v)[..WORDS], &qc.planes, WORDS)))
            .sum()
    };
    let neon_row = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| prefetch(srow(v)));
        batch
            .iter()
            .map(|&v| i64::from(raw_one(&srow(v)[..WORDS], &qc.planes, WORDS)))
            .sum()
    };
    let mut gathered = vec![0u64; DEG * WORDS];
    let mut out = vec![0u32; DEG];
    let neon_batch = |batch: &[u32]| -> i64 {
        // SAFETY-free gather: copy the 64 rows into one contiguous block, then one
        // `raw_estimates` pass (the FastScan-style form).
        let mut g = gathered.clone();
        let mut o = out.clone();
        batch.iter().for_each(|&v| prefetch(srow(v)));
        for (i, &v) in batch.iter().enumerate() {
            g[i * WORDS..(i + 1) * WORDS].copy_from_slice(&srow(v)[..WORDS]);
        }
        raw_estimates(&g, &qc.planes, WORDS, &mut o);
        o.iter().map(|&r| i64::from(r)).sum()
    };
    // Loop models: screen every fresh neighbour, then estimate the survivors (about 24%
    // on nytimes, held-out: 76% skipped). `late` prefetches a survivor's code when it
    // survives (the committed loop); `early` prefetches every code with the screen rows.
    let mut sample: Vec<u32> = ids[..4096]
        .iter()
        .map(|&v| raw_one(&srow(v)[..WORDS], &qc.planes, WORDS))
        .collect();
    sample.sort_unstable();
    let cut = sample[sample.len() * 24 / 100];
    let survive = |v: u32| raw_one(&srow(v)[..WORDS], &qc.planes, WORDS) < cut;
    let late = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| prefetch(srow(v)));
        let mut acc = 0i64;
        for &v in batch {
            if survive(v) {
                prefetch_bytes(code(v));
                acc += i64::from(dot_i8_inline(&q, code(v)));
            }
        }
        acc
    };
    let late_two_pass = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| prefetch(srow(v)));
        let survivors: Vec<u32> = batch.iter().copied().filter(|&v| survive(v)).collect();
        survivors.iter().for_each(|&v| prefetch_bytes(code(v)));
        survivors
            .iter()
            .map(|&v| i64::from(dot_i8_inline(&q, code(v))))
            .sum()
    };
    let early = |batch: &[u32]| -> i64 {
        batch.iter().for_each(|&v| {
            prefetch(srow(v));
            prefetch_bytes(code(v));
        });
        batch
            .iter()
            .filter(|&&v| survive(v))
            .map(|&v| i64::from(dot_i8_inline(&q, code(v))))
            .sum()
    };
    let (t_est, a) = time(&est);
    let (t_scalar, b) = time(&scalar);
    let (t_neon, c) = time(&neon_row);
    let (t_batch, d) = time(&neon_batch);
    let (t_late, e) = time(&late);
    let (t_late2, e2) = time(&late_two_pass);
    let (t_early, f) = time(&early);
    gathered.clear();
    out.clear();
    assert_eq!(c, d);
    assert_eq!(e, f);
    assert_eq!(e, e2);
    let survivors = ids.iter().filter(|&&v| survive(v)).count() as f64 / ids.len() as f64;
    let report = format!(
        "{{\"n_rows\": {N}, \"dim\": {DIM}, \"degree\": {DEG}, \"expansions\": {EXPANSIONS}, \
         \"ns_per_neighbour\": {{\"est_8bit_sdot\": {t_est:.2}, \"screen_scalar\": {t_scalar:.2}, \
         \"screen_neon_row\": {t_neon:.2}, \"screen_neon_gathered_batch\": {t_batch:.2}}}, \
         \"ratio_to_est\": {{\"screen_scalar\": {:.3}, \"screen_neon_row\": {:.3}, \
         \"screen_neon_gathered_batch\": {:.3}}}, \"kill_rule\": \"screen < 0.25 of est\", \
         \"loop_models_ns_per_fresh_neighbour\": {{\"estimate_all\": {t_est:.2}, \
         \"screen_then_late_prefetch\": {t_late:.2}, \"screen_then_two_pass_prefetch\": {t_late2:.2}, \
         \"screen_with_early_prefetch\": {t_early:.2}}}, \"survivor_share\": {survivors:.3}, \
         \"checksums\": [{a}, {b}, {c}, {e}]}}",
        t_scalar / t_est,
        t_neon / t_est,
        t_batch / t_est
    );
    println!("{report}");
    if let Ok(path) = crate::compatibility::var("WARM_SCREEN_BENCH_OUT") {
        std::fs::write(path, format!("{report}\n")).unwrap();
    }
}

type ScreenRow = fn(&[u64], &[u64], usize) -> u32;
type ScreenBlock = fn(&[u64], &[u64], usize, &mut [u32]);

/// Every 1-bit screen kernel this CPU can run, by name: `(name, one row, rows)`. The first
/// is the scalar reference, `dispatch` is what the index calls.
fn screen_variants() -> Vec<(&'static str, ScreenRow, ScreenBlock)> {
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_mut))]
    let mut v: Vec<(&'static str, ScreenRow, ScreenBlock)> = vec![
        ("scalar", raw_one_scalar, raw_scalar),
        ("dispatch", raw_one, raw_estimates),
    ];
    // SAFETY (every unsafe kernel below): its target features are detected before it is
    // listed; the kernels check their slice lengths themselves.
    #[cfg(target_arch = "x86_64")]
    {
        let popcnt = is_x86_feature_detected!("popcnt");
        let avx512 =
            popcnt && is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl");
        if popcnt {
            v.push((
                "popcnt",
                |c, p, w| {
                    assert_eq!(p.len(), QUERY_BITS * w);
                    unsafe { x86::raw_one_popcnt(c, p, w) }
                },
                |c, p, w, o| unsafe { x86::raw_popcnt(c, p, w, o) },
            ));
        }
        if popcnt && is_x86_feature_detected!("avx2") {
            v.push((
                "avx2",
                |c, p, w| unsafe { x86::raw_one_avx2(c, p, w) },
                |c, p, w, o| unsafe { x86::raw_avx2(c, p, w, o) },
            ));
        }
        if avx512 && is_x86_feature_detected!("avx512vpopcntdq") {
            v.push((
                "avx512_vpopcntdq",
                |c, p, w| unsafe { x86::raw_one_vpopcntdq(c, p, w) },
                |c, p, w, o| unsafe { x86::raw_vpopcntdq(c, p, w, o) },
            ));
        }
        if avx512
            && is_x86_feature_detected!("avx512bw")
            && is_x86_feature_detected!("avx512bitalg")
        {
            v.push((
                "avx512_bitalg",
                |c, p, w| unsafe { x86::raw_one_bitalg(c, p, w) },
                |c, p, w, o| unsafe { x86::raw_bitalg(c, p, w, o) },
            ));
        }
    }
    v
}

/// Every screen kernel against the scalar reference, bit for bit. Row widths 0..=33 words
/// bracket every vector width (2 words for SSE, 4 for AVX2 and ymm, 8 for zmm, 16 and 32
/// for the unrolled multiples) at limit-1, limit and limit+1, so each tail path runs. Rows
/// start 0..8 words into their buffer: with `&[u64]` an 8-byte step is the only possible
/// misalignment, and eight steps cover every offset modulo 64 bytes. All-ones rows hit
/// the per-byte maximum of the u8 accumulators.
#[test]
fn every_screen_kernel_matches_scalar() {
    let variants = screen_variants();
    let names: Vec<_> = variants.iter().map(|v| v.0).collect();
    eprintln!(
        "screen kernels under test on {}: {names:?}",
        std::env::consts::ARCH
    );
    let mut rng = SplitMix(23);
    let rows = 5;
    for words in 0..=33usize {
        for offset in 0..8 {
            for round in 0..4 {
                let mut cbuf: Vec<u64> =
                    (0..offset + rows * words).map(|_| rng.next_u64()).collect();
                let mut pbuf: Vec<u64> = (0..offset + QUERY_BITS * words)
                    .map(|_| rng.next_u64())
                    .collect();
                match round {
                    0 => {
                        cbuf.fill(u64::MAX);
                        pbuf.fill(u64::MAX);
                    }
                    1 => cbuf.fill(u64::MAX),
                    2 => pbuf.fill(0),
                    _ => {}
                }
                let (codes, planes) = (&cbuf[offset..], &pbuf[offset..]);
                let want: Vec<u32> = (0..rows)
                    .map(|r| raw_one_scalar(&codes[r * words..][..words], planes, words))
                    .collect();
                for (name, row, block) in &variants {
                    for (r, &w) in want.iter().enumerate() {
                        let got = row(&codes[r * words..][..words], planes, words);
                        assert_eq!(got, w, "{name} row, words {words}, offset {offset}");
                    }
                    if words > 0 {
                        let mut got = vec![u32::MAX; rows];
                        block(codes, planes, words, &mut got);
                        assert_eq!(got, want, "{name} rows, words {words}, offset {offset}");
                    }
                }
            }
        }
    }
}

/// Kill test for the 1-bit screen popcount kernels, paired: one binary, the variants
/// timed in alternation inside every round (order rotated per round), and the reported
/// figure is the per-round ratio to `scalar` and to the incumbent (`popcnt` on x86, which
/// is what dispatch ran before; `scalar` elsewhere): median and range over the rounds.
/// Shapes per width (2, 4, 8, 16 words = 128 to 1024 dims): `row_cold`, one call per
/// random row of a 64 MiB table, prefetched in batches of 64 (the `vertex.rs` shape);
/// `block_hot`, one call over a 64-row block in L1 (the `raw_estimates` shape of the
/// search loop); `block_cold`, the same over random blocks of the table. Rows start one
/// word into their buffer (unaligned). Every call goes through a fn pointer, the same for
/// every variant.
/// `cargo test --release --lib screen_popcount_microbench -- --ignored --nocapture`;
/// writes JSON to `$WARM_SCREEN_POPCOUNT_BENCH_OUT` when set; `WARM_BENCH_ROUNDS` (default 11).
#[test]
#[ignore = "microbenchmark: run in release with --ignored --nocapture"]
fn screen_popcount_microbench() {
    use std::time::Instant;
    const TABLE_BYTES: usize = 64 << 20;
    const CALLS: usize = 1 << 20;
    const DEG: usize = 64;
    let rounds: usize = crate::compatibility::var("WARM_BENCH_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(11);
    let variants = screen_variants();
    let incumbent = if variants.iter().any(|v| v.0 == "popcnt") {
        "popcnt"
    } else {
        "scalar"
    };
    let mut rng = SplitMix(5);
    let median = |mut x: Vec<f64>| -> (f64, f64, f64) {
        x.sort_by(f64::total_cmp);
        (x[x.len() / 2], x[0], x[x.len() - 1])
    };
    let mut report = Vec::new();
    for words in [2usize, 4, 8, 16] {
        let n = TABLE_BYTES / (words * 8);
        let buf: Vec<u64> = (0..1 + n * words).map(|_| rng.next_u64()).collect();
        let table = &buf[1..];
        let planes: Vec<u64> = (0..QUERY_BITS * words).map(|_| rng.next_u64()).collect();
        let ids: Vec<u32> = (0..CALLS)
            .map(|_| (rng.next_u64() % n as u64) as u32)
            .collect();
        let blocks: Vec<u32> = (0..CALLS / DEG)
            .map(|_| (rng.next_u64() % (n / DEG) as u64) as u32)
            .collect();
        let hot = &table[..DEG * words];
        let row = |v: u32| &table[v as usize * words..][..words];
        let block = |b: u32| &table[b as usize * DEG * words..][..DEG * words];
        // ns per row: [shape][variant][round]
        let mut ns = vec![vec![Vec::with_capacity(rounds); variants.len()]; 3];
        let mut sums = vec![[0u64; 3]; variants.len()];
        let mut out = vec![0u32; DEG];
        for round in 0..rounds {
            for j in 0..variants.len() {
                let i = (j + round) % variants.len();
                let (_, k_row, k_block) = variants[i];
                let t = Instant::now();
                let mut s = 0u64;
                for batch in ids.as_chunks::<DEG>().0 {
                    batch.iter().for_each(|&v| prefetch(row(v)));
                    for &v in batch {
                        s += u64::from(k_row(row(v), &planes, words));
                    }
                }
                ns[0][i].push(t.elapsed().as_secs_f64() * 1e9 / CALLS as f64);
                sums[i][0] = s;
                let t = Instant::now();
                let mut s = 0u64;
                for _ in 0..CALLS / DEG {
                    k_block(std::hint::black_box(hot), &planes, words, &mut out);
                    s += out.iter().map(|&r| u64::from(r)).sum::<u64>();
                }
                ns[1][i].push(t.elapsed().as_secs_f64() * 1e9 / CALLS as f64);
                sums[i][1] = s;
                let t = Instant::now();
                let mut s = 0u64;
                for &b in &blocks {
                    k_block(block(b), &planes, words, &mut out);
                    s += out.iter().map(|&r| u64::from(r)).sum::<u64>();
                }
                ns[2][i].push(t.elapsed().as_secs_f64() * 1e9 / CALLS as f64);
                sums[i][2] = s;
            }
        }
        for (i, (name, _, _)) in variants.iter().enumerate() {
            assert_eq!(sums[i], sums[0], "{name} checksum, words {words}");
        }
        let inc = variants.iter().position(|v| v.0 == incumbent).unwrap();
        for (s, shape) in ["row_cold", "block_hot", "block_cold"].iter().enumerate() {
            for (i, (name, _, _)) in variants.iter().enumerate() {
                let ratio = |base: usize| {
                    median((0..rounds).map(|r| ns[s][i][r] / ns[s][base][r]).collect())
                };
                let (t, t_lo, t_hi) = median(ns[s][i].clone());
                let (rs, rs_lo, rs_hi) = ratio(0);
                let (ri, ri_lo, ri_hi) = ratio(inc);
                report.push(format!(
                    "{{\"words\": {words}, \"dim\": {}, \"shape\": \"{shape}\", \"variant\": \"{name}\", \
                     \"ns_per_row_median\": {t:.3}, \"ns_per_row_range\": [{t_lo:.3}, {t_hi:.3}], \
                     \"ratio_to_scalar_median\": {rs:.3}, \"ratio_to_scalar_range\": [{rs_lo:.3}, {rs_hi:.3}], \
                     \"ratio_to_incumbent_median\": {ri:.3}, \"ratio_to_incumbent_range\": [{ri_lo:.3}, {ri_hi:.3}]}}",
                    words * 64
                ));
            }
        }
    }
    let json = format!(
        "{{\"arch\": \"{}\", \"rounds\": {rounds}, \"incumbent\": \"{incumbent}\", \"results\": [\n  {}\n]}}\n",
        std::env::consts::ARCH,
        report.join(",\n  ")
    );
    println!("{json}");
    if let Ok(path) = crate::compatibility::var("WARM_SCREEN_POPCOUNT_BENCH_OUT") {
        std::fs::write(path, json).unwrap();
    }
}

type I8Kernel = fn(&[i8], &[i8]) -> i32;

/// Every integer kernel this CPU can run, by name: `(name, i8 dot, i4 dot)`.
fn int_variants() -> Vec<(&'static str, I8Kernel, I8Kernel)> {
    // SAFETY (every unsafe kernel below): its target features are detected before it is
    // listed, and every caller passes rows of the lengths the kernel requires.
    let mut v: Vec<(&'static str, I8Kernel, I8Kernel)> =
        vec![("scalar", dot_i8_scalar, dot_i4_scalar)];
    #[cfg(target_arch = "aarch64")]
    {
        v.push(("neon_widening", neon::dot_i8, neon::dot_i4));
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            v.push(("sdot", neon::dot_i8_sdot, |q, p| unsafe {
                neon::sdot4(q, p)
            }));
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            v.push((
                "avx2",
                |a, b| unsafe { x86::dot_i8_avx2(a, b) },
                |q, p| unsafe { x86::dot_i4_avx2(q, p) },
            ));
        }
        if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") {
            v.push((
                "vnni",
                |a, b| unsafe { x86::dot_i8_vnni(a, b) },
                |q, p| unsafe { x86::dot_i4_vnni(q, p) },
            ));
        }
    }
    v
}

fn random_i8(rng: &mut SplitMix, n: usize) -> Vec<i8> {
    (0..n).map(|_| rng.next_u64() as i8).collect()
}

/// Packs signed nibbles (`x >> 4` of each byte) two per byte as [`dot_f32_i4`] reads them.
fn pack_nibbles(bytes: &[i8]) -> Vec<i8> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (((p[0] >> 4) as u8 & 0x0F) | (((p[1] >> 4) as u8) << 4)) as i8)
        .collect()
}

/// The empty-row, tail, odd-dimension and extreme-value cases every integer kernel must
/// match the scalar reference on, bit for bit (lengths are the multiples of 16 the index
/// produces).
#[test]
fn every_integer_kernel_matches_scalar() {
    let mut rng = SplitMix(99);
    for (name, i8k, i4k) in int_variants() {
        for len in (0..=512).step_by(16) {
            for round in 0..24 {
                let mut a = random_i8(&mut rng, len);
                let mut b = random_i8(&mut rng, len);
                match round {
                    0 => {
                        a.fill(-128);
                        b.fill(-128);
                    }
                    1 => {
                        a.fill(127);
                        b.fill(-128);
                    }
                    2 => {
                        a.fill(-128);
                        b.fill(127);
                    }
                    3 => {
                        a.fill(0);
                        b.fill(-128);
                    }
                    _ => {}
                }
                assert_eq!(i8k(&a, &b), dot_i8_scalar(&a, &b), "{name} i8 len {len}");
                if len % 32 == 0 {
                    let mut packed = pack_nibbles(&b);
                    if round == 1 {
                        packed.fill(-128); // both nibbles -8
                    } else if round == 2 {
                        packed.fill(0x77); // both nibbles 7
                    }
                    let mut q = vec![0i8; len];
                    interleave_for_i4(&a, &mut q);
                    assert_eq!(
                        i4k(&q, &packed),
                        dot_i4_scalar(&q, &packed),
                        "{name} i4 len {len}"
                    );
                }
            }
        }
    }
}

/// Kill test for every distance kernel: ns per call with the search loop's access
/// pattern (random fresh rows, prefetched in batches of 64) on a working set far larger
/// than cache (`cold`), and over rows that stay in L1 (`hot`, the compute bound), for
/// the row widths the indexes use. A kernel that does not beat the incumbent is dropped.
/// `cargo test --release dot_kernel_microbench -- --ignored --nocapture`; writes JSON to
/// `$WARM_PLATFORM_BENCH_OUT` when set.
#[test]
#[ignore = "microbenchmark: run in release with --ignored --nocapture"]
fn dot_kernel_microbench() {
    use std::time::Instant;
    const COLD_ROWS: usize = 400_000;
    const CALLS: usize = 1 << 20;
    let mut rng = SplitMix(3);
    let mut report = Vec::new();
    let mut push = |dim: usize, kind: &str, name: &str, cold: f64, hot: f64| {
        report.push(format!(
            "{{\"dim\": {dim}, \"kernel\": \"{kind}\", \"variant\": \"{name}\", \"ns_cold\": {cold:.3}, \"ns_hot\": {hot:.3}}}"
        ));
    };
    for dim in [128usize, 256] {
        let cold = random_i8(&mut rng, COLD_ROWS * dim);
        let ids: Vec<u32> = (0..CALLS)
            .map(|_| (rng.next_u64() % COLD_ROWS as u64) as u32)
            .collect();
        let hot = random_i8(&mut rng, 64 * dim);
        let q = random_i8(&mut rng, dim);
        let mut qi = vec![0i8; dim];
        interleave_for_i4(&q, &mut qi);
        let cold4 = pack_nibbles(&cold);
        let hot4 = pack_nibbles(&hot);
        for (kind, width) in [("i8", dim), ("i4", dim / 2)] {
            let (rows_cold, rows_hot): (&[i8], &[i8]) = if kind == "i8" {
                (&cold, &hot)
            } else {
                (&cold4, &hot4)
            };
            let query: &[i8] = if kind == "i8" { &q } else { &qi };
            let mut ref_sum = None;
            for (name, i8k, i4k) in int_variants() {
                let k = if kind == "i8" { i8k } else { i4k };
                let mut best = (f64::INFINITY, f64::INFINITY);
                let mut sum = 0i64;
                for _ in 0..7 {
                    let t = Instant::now();
                    let mut s = 0i64;
                    for batch in ids.chunks(64) {
                        for &v in batch {
                            prefetch_bytes(&rows_cold[v as usize * width..][..width]);
                        }
                        for &v in batch {
                            s += i64::from(k(query, &rows_cold[v as usize * width..][..width]));
                        }
                    }
                    best.0 = best.0.min(t.elapsed().as_secs_f64() * 1e9 / CALLS as f64);
                    sum = s;
                    let t = Instant::now();
                    let mut s = 0i64;
                    for _ in 0..CALLS / 64 {
                        for r in rows_hot.chunks_exact(width) {
                            s += i64::from(k(query, r));
                        }
                    }
                    best.1 = best.1.min(t.elapsed().as_secs_f64() * 1e9 / CALLS as f64);
                    std::hint::black_box(s);
                }
                assert_eq!(*ref_sum.get_or_insert(sum), sum, "{name} {kind} dim {dim}");
                push(dim, kind, name, best.0, best.1);
            }
        }
    }
    let json = format!(
        "{{\"arch\": \"{}\", \"results\": [\n  {}\n]}}\n",
        std::env::consts::ARCH,
        report.join(",\n  ").replace("NaN", "null")
    );
    println!("{json}");
    if let Ok(path) = crate::compatibility::var("WARM_PLATFORM_BENCH_OUT") {
        std::fs::write(path, json).unwrap();
    }
}
