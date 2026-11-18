// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn data(n: usize, dim: usize, clusters: usize, seed: u64) -> Vec<f32> {
    let mut r = SplitMix(seed);
    let mut u = move || (r.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
    let centres: Vec<f32> = (0..clusters * dim).map(|_| 8.0 * u()).collect();
    let mut out = Vec::with_capacity(n * dim);
    for i in 0..n {
        let c = i % clusters;
        for t in 0..dim {
            out.push(centres[c * dim + t] + u());
        }
    }
    out
}

fn naive(rows: &[(u32, &[f32])], q: &[f32], k: usize) -> Vec<u32> {
    let mut d: Vec<(f32, u32)> = rows.iter().map(|(id, r)| (sq(q, r), *id)).collect();
    d.sort_by(|a, b| a.0.total_cmp(&b.0));
    d.into_iter().take(k).map(|x| x.1).collect()
}

fn recall(a: &[Vec<u32>], b: &[Vec<u32>]) -> f64 {
    let hit: usize = a
        .iter()
        .zip(b)
        .map(|(x, y)| x.iter().filter(|i| y.contains(i)).count())
        .sum();
    hit as f64 / b.iter().map(Vec::len).sum::<usize>() as f64
}

/// Every live slot's two memberships point at list positions holding that slot, and the
/// lists hold exactly two memberships per live row.
fn check_lists(t: &PartitionTier) {
    let tr = t.trained.as_ref().unwrap();
    let total: usize = tr.lists.iter().map(|l| l.slots.len()).sum();
    let mut memberships = 0;
    for &slot in t.id_slot.values() {
        let e = tr.loc[slot as usize];
        assert_ne!(e[0].0, u32::MAX, "no primary list");
        assert_ne!(e[0].0, e[1].0, "spill list equals primary");
        for (list, pos) in e.into_iter().filter(|x| x.0 != u32::MAX) {
            memberships += 1;
            assert_eq!(tr.lists[list as usize].slots[pos as usize], slot);
        }
    }
    assert_eq!(total, memberships);
    if t.params.spill_eps.is_infinite() {
        assert_eq!(total, 2 * t.len());
    }
    for (li, l) in tr.lists.iter().enumerate() {
        assert_eq!(
            l.codes.len(),
            l.slots.len().div_ceil(BLOCK) * t.shape.stride()
        );
        let c = &tr.centres[li * t.dim..(li + 1) * t.dim];
        let (mut s1, mut s2) = (0f64, 0f64);
        for &slot in &l.slots {
            let d = f64::from(sq(t.row(slot), c).sqrt());
            (s1, s2) = (s1 + d, s2 + d * d);
        }
        assert!(
            (l.dist_sum - s1).abs() <= 1e-6 * (1.0 + s1),
            "list {li} dist_sum"
        );
        assert!(
            (l.dist_sq - s2).abs() <= 1e-6 * (1.0 + s2),
            "list {li} dist_sq"
        );
    }
}

/// Distances 1.0, 1.2, 1.5, 2.0 (squared 1, 1.44, 2.25, 4), lists in that order.
fn ladder() -> Vec<f32> {
    vec![4.0, 1.0, 2.25, 1.44]
}

#[test]
fn select_probes_rules_respect_bounds_and_cutoffs() {
    let d2 = ladder();
    let sel = |rule, nmin, nmax| select_probes(&d2, &[0.0; 4], rule, nmin, nmax);
    // Ratio: d_i^2 <= d_1^2 (1+eps) keeps 1 and 1.44 at eps 0.5.
    assert_eq!(sel(ProbeRule::Ratio { eps: 0.5 }, 1, 4), [1, 3]);
    assert_eq!(sel(ProbeRule::Ratio { eps: 0.0 }, 1, 4), [1]);
    // Floor and cap.
    assert_eq!(sel(ProbeRule::Ratio { eps: 0.0 }, 3, 4), [1, 3, 2]);
    assert_eq!(sel(ProbeRule::Ratio { eps: 9.0 }, 1, 2), [1, 3]);
    // Monotone in eps.
    let mut prev = 0;
    for e in 0..40 {
        let n = sel(
            ProbeRule::Ratio {
                eps: e as f32 * 0.1,
            },
            1,
            4,
        )
        .len();
        assert!(n >= prev);
        prev = n;
    }
    // Radius: with zero radii the cut is d_i <= (1+eps) d_1 = 1.3, keeping 1.0 and 1.2.
    assert_eq!(
        sel(
            ProbeRule::Radius {
                eps: 0.3,
                scale: 1.0
            },
            1,
            4
        ),
        [1, 3]
    );
    // A large radius on the farthest list (d = 2) keeps it past the nearer, small-radius list
    // with d = 1.5; scale 0 ignores radii.
    let radii = [1.0, 0.0, 0.0, 0.0];
    let r = |scale| select_probes(&d2, &radii, ProbeRule::Radius { eps: 0.3, scale }, 1, 4);
    assert_eq!(r(1.0), [1, 3, 0]);
    assert_eq!(r(0.0), [1, 3]);
}

/// With a cutoff that never bites, an adaptive rule probes the same lists as `Fixed` and so
/// returns identical results; `Fixed` ignores the rule fields.
#[test]
fn adaptive_rules_without_a_cut_match_fixed() {
    let (n, dim, k) = (3000, 16, 10);
    let x = data(n, dim, 25, 1);
    let q = data(40, dim, 25, 2);
    let ids: Vec<u32> = (0..n as u32).collect();
    let mut t = PartitionTier::new(
        dim,
        PartitionParams {
            nprobe: 7,
            reorder: 40,
            ..params(20)
        },
    )
    .unwrap();
    t.insert(&ids, &x).unwrap();
    let fixed = t.search(&q, k).unwrap();
    let pf = t.last_search_profile();
    assert_eq!(pf.probes, 40 * 7);
    t.set_probe_rule(ProbeRule::Fixed, 5);
    assert_eq!(t.search(&q, k).unwrap(), fixed);
    for rule in [
        ProbeRule::Ratio { eps: 1e30 },
        ProbeRule::Radius {
            eps: 1e30,
            scale: 1.0,
        },
    ] {
        t.set_probe_rule(rule, 1);
        assert_eq!(t.search(&q, k).unwrap(), fixed, "{rule:?}");
        assert_eq!(t.last_search_profile().probes, pf.probes);
    }
    // A tight cut probes fewer lists, never fewer than the floor.
    t.set_probe_rule(ProbeRule::Ratio { eps: 0.0 }, 2);
    t.search(&q, k).unwrap();
    let tight = t.last_search_profile().probes;
    assert!((40 * 2..40 * 7).contains(&(tight as usize)), "{tight}");
}

fn params(lists: usize) -> PartitionParams {
    PartitionParams {
        lists,
        train_at: 1000,
        train_sample: 2000,
        pq_m: 8,
        pq_sample: 2000,
        nprobe: lists,
        reorder: 4000,
        ..PartitionParams::default()
    }
}

#[test]
fn full_probe_matches_exact_through_deletes_and_reinserts() {
    let (n, dim, k) = (3000, 16, 10);
    let x = data(n, dim, 25, 1);
    let q = data(40, dim, 25, 2);
    let mut t = PartitionTier::new(dim, params(20)).unwrap();
    let ids: Vec<u32> = (0..n as u32).map(|i| i * 7 + 3).collect();
    t.insert(&ids[..500], &x[..500 * dim]).unwrap();
    // Below train_at: exact brute force.
    let got = t.search(&q, k).unwrap();
    assert!(!t.is_trained());
    let rows: Vec<(u32, &[f32])> = (0..500)
        .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
        .collect();
    let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect();
    assert_eq!(recall(&got, &want), 1.0);
    t.insert(&ids[500..], &x[500 * dim..]).unwrap();
    let got = t.search(&q, k).unwrap();
    assert!(t.is_trained());
    check_lists(&t);
    let rows: Vec<(u32, &[f32])> = (0..n)
        .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
        .collect();
    let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect();
    assert_eq!(recall(&got, &want), 1.0);
    // Delete every third row, reinsert a third of those, check invariants and exactness.
    let del: Vec<u32> = ids.iter().copied().step_by(3).collect();
    assert_eq!(t.delete(&del), del.len());
    assert_eq!(t.delete(&del[..5]), 0);
    check_lists(&t);
    let back: Vec<usize> = (0..n).step_by(9).collect();
    let bid: Vec<u32> = back.iter().map(|&i| ids[i]).collect();
    let bx: Vec<f32> = back
        .iter()
        .flat_map(|&i| x[i * dim..(i + 1) * dim].to_vec())
        .collect();
    t.insert(&bid, &bx).unwrap();
    assert!(t.insert(&bid[..1], &bx[..dim]).is_err());
    check_lists(&t);
    let live: Vec<(u32, &[f32])> = (0..n)
        .filter(|i| i % 3 != 0 || i % 9 == 0)
        .map(|i| (ids[i], &x[i * dim..(i + 1) * dim]))
        .collect();
    assert_eq!(live.len(), t.len());
    let want: Vec<Vec<u32>> = q.chunks(dim).map(|qq| naive(&live, qq, k)).collect();
    assert_eq!(recall(&t.search(&q, k).unwrap(), &want), 1.0);
    assert_eq!(recall(&t.search_exact(&q, k), &want), 1.0);
}

/// Starting from centres that sit on a small part of the data (a stale set), a few Lloyd
/// iterations with rebalancing give clusters that are more even than without it.
#[test]
fn rebalancing_kmeans_evens_out_stale_centres() {
    let (n, dim, k) = (6000, 8, 40);
    let x = data(n, dim, 8, 5);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| x[i * dim].total_cmp(&x[j * dim]));
    let stale: Vec<f32> = order[..k]
        .iter()
        .flat_map(|&i| x[i * dim..(i + 1) * dim].to_vec())
        .collect();
    let spread = |c: &[f32]| -> f64 {
        let c2: Vec<f32> = c.chunks(dim).map(|r| dot(r, r)).collect();
        let a = nearest_all(&x, dim, c, &c2);
        let mut counts = vec![0f64; k];
        for &ai in &a {
            counts[ai as usize] += 1.0;
        }
        counts.iter().map(|v| v * v).sum::<f64>() * k as f64 / (n as f64 * n as f64)
    };
    let plain = kmeans_from(&x, dim, stale.clone(), 3, false, &mut SplitMix(1));
    let even = kmeans_from(&x, dim, stale, 3, true, &mut SplitMix(1));
    assert!(
        spread(&even) < spread(&plain),
        "{} vs {}",
        spread(&even),
        spread(&plain)
    );
}

#[test]
fn nprobe_scales_piecewise_linearly_with_live_size() {
    let dim = 4;
    let mut t = PartitionTier::new(
        dim,
        PartitionParams {
            lists: 100,
            nprobe: 40,
            pq_m: 2,
            nprobe_by_live: vec![(100, 2.0), (300, 1.0), (500, 0.5)],
            ..PartitionParams::default()
        },
    )
    .unwrap();
    let x = vec![0.5f32; 600 * dim];
    let ids: Vec<u32> = (0..600).collect();
    let mut seen = Vec::new();
    for upto in [50usize, 100, 200, 300, 400, 500, 600] {
        let have = t.len();
        t.insert(&ids[have..upto], &x[have * dim..upto * dim])
            .unwrap();
        seen.push(t.effective_nprobe());
    }
    // 40 * (2.0, 2.0, 1.5, 1.0, 0.75, 0.5, 0.5), capped by the list count.
    assert_eq!(seen, vec![80, 80, 60, 40, 30, 20, 20]);
}

/// Retraining (scheduled by live size, by imbalance, cold or warm) re-places every live row:
/// the list invariants hold and a full probe still equals exact search after deletes.
#[test]
fn retrain_keeps_every_live_row_searchable() {
    let (n, dim, k) = (4000, 16, 10);
    let x = data(n, dim, 25, 3);
    let q = data(30, dim, 25, 4);
    let ids: Vec<u32> = (0..n as u32).collect();
    let want_for = |live: &[usize]| -> Vec<Vec<u32>> {
        let rows: Vec<(u32, &[f32])> = live
            .iter()
            .map(|&i| (ids[i], &x[i * dim..(i + 1) * dim]))
            .collect();
        q.chunks(dim).map(|qq| naive(&rows, qq, k)).collect()
    };
    for (warm, imb) in [(0, 0.0f32), (2, 0.0), (2, 1.0001)] {
        let mut t = PartitionTier::new(
            dim,
            PartitionParams {
                retrain_at: vec![2000, 3000],
                retrain_imb: imb,
                retrain_min_inserts: 500,
                retrain_warm_iters: warm,
                retrain_sample: 1500,
                spill_eps: if imb > 0.0 { 0.25 } else { f32::INFINITY },
                ..params(20)
            },
        )
        .unwrap();
        t.insert(&ids[..1200], &x[..1200 * dim]).unwrap();
        t.search(&q, k).unwrap();
        assert_eq!((t.trained_live, t.retrain_next), (1200, 0));
        t.insert(&ids[1200..2500], &x[1200 * dim..2500 * dim])
            .unwrap();
        let got = t.search(&q, k).unwrap();
        // The 2000 threshold fired once; the imbalance rule may add retrains.
        assert!(
            t.trained_live >= 2500 && t.retrain_next >= 1,
            "warm {warm} imb {imb}"
        );
        check_lists(&t);
        assert_eq!(recall(&got, &want_for(&(0..2500).collect::<Vec<_>>())), 1.0);
        let del: Vec<u32> = ids[..2500].iter().copied().step_by(4).collect();
        t.delete(&del);
        t.insert(&ids[2500..], &x[2500 * dim..]).unwrap();
        let got = t.search(&q, k).unwrap();
        check_lists(&t);
        let live: Vec<usize> = (0..n).filter(|&i| i >= 2500 || i % 4 != 0).collect();
        assert_eq!(live.len(), t.len());
        assert_eq!(recall(&got, &want_for(&live)), 1.0);
    }
}

/// The quad layout (AVX-512 VBMI/VNNI) and the LUT16 layout give the same results, blocks
/// and admissions through deletes and reinserts, with a reorder small enough that the
/// running threshold prunes and an `m` that pads the last quad.
#[test]
fn score_key_orders_like_total_cmp_and_round_trips() {
    let vals = [
        f32::NEG_INFINITY,
        -3.5,
        -0.0,
        0.0,
        1e-30,
        2.0,
        7e9,
        f32::INFINITY,
    ];
    for &a in &vals {
        assert_eq!(key_score(score_key(a, 9)).to_bits(), a.to_bits());
        for &b in &vals {
            assert_eq!(
                score_key(a, 5).cmp(&score_key(b, 5)),
                a.total_cmp(&b),
                "{a} vs {b}"
            );
        }
    }
    assert!(score_key(1.0, 3) < score_key(1.0, 4));
    assert_eq!(score_key(1.5, 77) as u32, 77);
}

#[test]
fn fused_smallest_k_equals_scalar() {
    let mut rng = SplitMix(11);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for n in [1usize, 5, 31, 32, 33, 39, 40, 41, 100, 997, 5000] {
        for k in [1usize, 7, 32, 64] {
            for quant in [0.0f32, 8.0] {
                let row: Vec<f32> = (0..n)
                    .map(|_| (rng.next() >> 40) as f32 / (1u64 << 24) as f32)
                    .collect();
                let c2: Vec<f32> = (0..n)
                    .map(|_| (rng.next() >> 40) as f32 / (1u64 << 23) as f32)
                    .collect();
                let q = |x: f32| {
                    if quant > 0.0 {
                        (x * quant).round() / quant
                    } else {
                        x
                    }
                };
                let (row, c2): (Vec<f32>, Vec<f32>) = (
                    row.into_iter().map(q).collect(),
                    c2.into_iter().map(q).collect(),
                );
                let d: Vec<f32> = c2.iter().zip(&row).map(|(c, v)| c - 2.0 * v).collect();
                smallest_k(&d, k, &mut a);
                smallest_k_centres(&row, &c2, k, &mut b);
                assert_eq!(a, b, "n {n} k {k}");
            }
        }
    }
}

#[test]
fn gemm_nt_matches_reference() {
    let mut rng = SplitMix(9);
    for (m, n, k) in [
        (1usize, 1usize, 1usize),
        (7, 33, 100),
        (130, 257, 100),
        (64, 70, 17),
    ] {
        let a: Vec<f32> = (0..m * k)
            .map(|_| (rng.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5)
            .collect();
        let b: Vec<f32> = (0..n * k)
            .map(|_| (rng.next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5)
            .collect();
        let mut c = vec![f32::NAN; m * n];
        gemm_nt(&a, m, &b, n, k, &mut c);
        for i in 0..m {
            for j in 0..n {
                let want: f64 = (0..k)
                    .map(|t| f64::from(a[i * k + t]) * f64::from(b[j * k + t]))
                    .sum();
                assert!(
                    (f64::from(c[i * n + j]) - want).abs() < 1e-4,
                    "{m}x{n}x{k} at {i},{j}"
                );
            }
        }
    }
}

#[test]
fn slot_set_agrees_with_a_hash_set() {
    let mut rng = SplitMix(9);
    for cap in [1usize, 7, 1000] {
        let mut set = SlotSet::new(cap);
        let mut want = std::collections::HashSet::new();
        for round in 0..3 {
            set.clear();
            want.clear();
            // `cap` distinct members at most, with repeats and clustered values.
            for _ in 0..4 * cap {
                let s = (rng.next() % (cap as u64).max(2)) as u32 * 4096 + round;
                if want.len() < cap || want.contains(&s) {
                    assert_eq!(set.insert(s), want.insert(s), "cap {cap}");
                }
            }
        }
    }
}

#[test]
fn quad_layout_equals_lut16_layout() {
    if !quad_available() {
        eprintln!("skipped: no AVX-512 VBMI/VNNI");
        return;
    }
    let (n, dim, k) = (3000, 40, 10);
    let x = data(n, dim, 25, 1);
    // Enough queries that each thread meets full chunks of `QB` and partial ones.
    let q = data(400, dim, 25, 2);
    let ids: Vec<u32> = (0..n as u32).map(|i| i * 5 + 1).collect();
    for m in [8usize, 10] {
        let mk = |quad: bool| {
            let mut t = PartitionTier::new(
                dim,
                PartitionParams {
                    pq_m: m,
                    nprobe: 8,
                    reorder: 40,
                    ..params(20)
                },
            )
            .unwrap();
            t.shape.quad = quad;
            t
        };
        let (mut a, mut b) = (mk(true), mk(false));
        for t in [&mut a, &mut b] {
            t.insert(&ids, &x).unwrap();
        }
        let same = |a: &mut PartitionTier, b: &mut PartitionTier| {
            let (ra, rb) = (a.search(&q, k).unwrap(), b.search(&q, k).unwrap());
            assert_eq!(ra, rb, "m {m}");
            let (pa, pb) = (a.last_search_profile(), b.last_search_profile());
            assert_eq!((pa.blocks, pa.admitted), (pb.blocks, pb.admitted), "m {m}");
        };
        // The first search trains both tiers.
        same(&mut a, &mut b);
        check_lists(&a);
        check_lists(&b);
        let del: Vec<u32> = ids.iter().copied().step_by(3).collect();
        for t in [&mut a, &mut b] {
            assert_eq!(t.delete(&del), del.len());
            check_lists(t);
            t.insert(&del[..200], &x[..200 * dim]).unwrap();
            check_lists(t);
        }
        same(&mut a, &mut b);
    }
}

#[test]
fn restricted_spill_equals_full_soar_when_all_centres_are_candidates() {
    let (n, dim, lists) = (2000, 16, 12);
    let x = data(n, dim, 30, 3);
    let mut p = params(lists);
    p.spill_k = lists;
    let mut t = PartitionTier::new(dim, p).unwrap();
    t.insert(&(0..n as u32).collect::<Vec<_>>(), &x).unwrap();
    t.train(None).unwrap();
    let tr = t.trained.as_ref().unwrap();
    let asg = tr.assign(&x, dim, lists, 1.0, f32::INFINITY);
    for (i, &(a1, a2)) in asg.iter().enumerate() {
        let xi: Vec<f64> = x[i * dim..(i + 1) * dim]
            .iter()
            .map(|&v| f64::from(v))
            .collect();
        let c = |j: usize| -> Vec<f64> {
            tr.centres[j * dim..(j + 1) * dim]
                .iter()
                .map(|&v| f64::from(v))
                .collect()
        };
        let d = |j: usize| -> f64 { xi.iter().zip(c(j)).map(|(a, b)| (a - b).powi(2)).sum() };
        let near = (0..lists).min_by(|&a, &b| d(a).total_cmp(&d(b))).unwrap();
        assert!(d(a1 as usize) <= d(near) * (1.0 + 1e-5) + 1e-6);
        let r: Vec<f64> = xi.iter().zip(c(a1 as usize)).map(|(a, b)| a - b).collect();
        let rr: f64 = r.iter().map(|v| v * v).sum::<f64>().max(1e-20);
        let loss = |j: usize| -> f64 {
            let xc: Vec<f64> = xi.iter().zip(c(j)).map(|(a, b)| a - b).collect();
            let pr: f64 = xc.iter().zip(&r).map(|(a, b)| a * b).sum();
            xc.iter().map(|v| v * v).sum::<f64>() + pr * pr / rr
        };
        let best = (0..lists)
            .filter(|&j| j != a1 as usize)
            .map(loss)
            .fold(f64::INFINITY, f64::min);
        assert_ne!(a2, a1);
        assert!(loss(a2 as usize) <= best * (1.0 + 1e-4) + 1e-6, "row {i}");
    }
}

/// The `i`-th token of a `;`-separated rule list as a JSON-key-safe label.
fn tokens(rules: &str, i: usize) -> String {
    rules.split(';').nth(i).unwrap().replace(':', "_")
}

/// A2 prefix acceptance (STATE.md 02:15Z): WARM_L22_ACC_{BASE,N,QUERIES,GT,CENTRES,OUT}.
/// Base is the msturing fbin (first N rows), queries `nq x 100` f32, GT `nq x 10` i32,
/// centres `5000 x 100` f32 (A2's, reproduced by seed). Prints recall at nprobe 350 and 700.
#[test]
#[ignore]
fn partition_prefix_acceptance() {
    use std::time::Instant;
    let var = |k: &str| crate::compatibility::var(&format!("WARM_L22_ACC_{k}")).unwrap();
    let f32s = |b: Vec<u8>| -> Vec<f32> {
        b.as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    };
    let dim = 100;
    let n: usize = var("N").parse().unwrap();
    let base = {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(var("BASE")).unwrap();
        f.seek(SeekFrom::Start(8)).unwrap();
        let mut b = vec![0u8; n * dim * 4];
        f.read_exact(&mut b).unwrap();
        f32s(b)
    };
    let q = f32s(std::fs::read(var("QUERIES")).unwrap());
    let gt: Vec<Vec<u32>> = std::fs::read(var("GT"))
        .unwrap()
        .as_chunks::<40>()
        .0
        .iter()
        .map(|r| {
            r.as_chunks::<4>()
                .0
                .iter()
                .map(|c| i32::from_le_bytes(*c) as u32)
                .collect()
        })
        .collect();
    let envn = |k: &str| -> Option<usize> {
        crate::compatibility::var(&format!("WARM_L22_ACC_{k}"))
            .ok()
            .map(|v| v.trim().parse().unwrap())
    };
    let defaults = PartitionParams::default();
    let lists = envn("LISTS").unwrap_or(defaults.lists);
    // Centres: a file (control), or k-means on the first TRAIN_N rows of TRAIN_BASE
    // (replay of a retrain at that state), or None = train at the evaluated state.
    let t0 = Instant::now();
    let centres: Option<Vec<f32>> = if let Ok(p) = crate::compatibility::var("WARM_L22_ACC_CENTRES")
    {
        Some(f32s(std::fs::read(p).unwrap()))
    } else if let Ok(tb) = crate::compatibility::var("WARM_L22_ACC_TRAIN_BASE") {
        use std::io::{Read, Seek, SeekFrom};
        let tn = envn("TRAIN_N").unwrap();
        let mut f = std::fs::File::open(tb).unwrap();
        f.seek(SeekFrom::Start(8)).unwrap();
        let mut b = vec![0u8; tn * dim * 4];
        f.read_exact(&mut b).unwrap();
        let rows = f32s(b);
        let mut rng = SplitMix(envn("SEED").map_or(defaults.seed, |v| v as u64));
        let idx = sample_indices(
            tn,
            envn("TRAIN_SAMPLE").unwrap_or(defaults.train_sample),
            &mut rng,
        );
        let mut sample = Vec::with_capacity(idx.len() * dim);
        for &i in &idx {
            sample.extend_from_slice(&rows[i * dim..(i + 1) * dim]);
        }
        let rebal = envn("REBAL") == Some(1);
        let stale = kmeans_with(
            &sample,
            dim,
            lists,
            envn("KM_ITERS").unwrap_or(defaults.kmeans_iters),
            rebal,
            &mut rng,
        );
        if let Some(wi) = envn("WARM_ITERS") {
            // Retrain warm-started from the stale centres on a sample of the evaluated state.
            let idx = sample_indices(
                n,
                envn("WARM_SAMPLE").unwrap_or(defaults.train_sample),
                &mut rng,
            );
            let mut ws = Vec::with_capacity(idx.len() * dim);
            for &i in &idx {
                ws.extend_from_slice(&base[i * dim..(i + 1) * dim]);
            }
            Some(kmeans_from(&ws, dim, stale, wi, rebal, &mut rng))
        } else {
            Some(stale)
        }
    } else {
        None
    };
    let kmeans_s = t0.elapsed().as_secs_f64();
    let mut t = PartitionTier::new(
        dim,
        PartitionParams {
            train_at: usize::MAX,
            lists,
            train_sample: envn("TRAIN_SAMPLE").unwrap_or(defaults.train_sample),
            kmeans_iters: envn("KM_ITERS").unwrap_or(defaults.kmeans_iters),
            spill_k: envn("SPILLK").unwrap_or(defaults.spill_k),
            seed: envn("SEED").map_or(defaults.seed, |v| v as u64),
            kmeans_rebalance: envn("REBAL") == Some(1),
            spill_eps: crate::compatibility::var("WARM_L22_ACC_SPILLEPS")
                .map_or(f32::INFINITY, |v| v.trim().parse().unwrap()),
            soar_lambda: crate::compatibility::var("WARM_L22_ACC_LAMBDA")
                .map_or(defaults.soar_lambda, |v| v.trim().parse().unwrap()),
            reorder: crate::compatibility::var("WARM_L22_ACC_REORDER")
                .map(|v| v.trim().parse().unwrap())
                .unwrap_or(PartitionParams::default().reorder),
            pq_m: crate::compatibility::var("WARM_L22_ACC_PQM")
                .map(|v| v.trim().parse().unwrap())
                .unwrap_or(PartitionParams::default().pq_m),
            ..PartitionParams::default()
        },
    )
    .unwrap();
    let t0 = Instant::now();
    t.insert(&(0..n as u32).collect::<Vec<_>>(), &base).unwrap();
    let insert_s = t0.elapsed().as_secs_f64();
    let t0 = Instant::now();
    t.train(centres).unwrap();
    let train_s = t0.elapsed().as_secs_f64();
    // RETRAIN=1: replay a warm retrain of the placed tier (stale centres above) with the
    // RT_ITERS / RT_SAMPLE / REBAL knobs and report its phases.
    let mut retrain_rep = String::new();
    if envn("RETRAIN") == Some(1) {
        t.params.retrain_warm_iters = envn("RT_ITERS").unwrap_or(3);
        t.params.retrain_sample = envn("RT_SAMPLE").unwrap_or(1_000_000);
        let t0 = Instant::now();
        t.train(None).unwrap();
        let ph = t.train_phases;
        retrain_rep = format!(
            ", \"retrain_s\": {:.3}, \"retrain_centres_s\": {:.3}, \"retrain_tables_s\": {:.3}, \"retrain_place_s\": {:.3}",
            t0.elapsed().as_secs_f64(),
            ph[0],
            ph[1],
            ph[2]
        );
    }
    let ls = {
        let mut v: Vec<usize> = t
            .trained
            .as_ref()
            .unwrap()
            .lists
            .iter()
            .map(|l| l.slots.len())
            .collect();
        v.sort_unstable();
        let tot: usize = v.iter().sum();
        let sq: f64 = v.iter().map(|&x| (x as f64).powi(2)).sum();
        format!(
            "{{\"entries\": {tot}, \"max\": {}, \"p99\": {}, \"p50\": {}, \"empty\": {}, \"imb\": {:.3}}}",
            v[v.len() - 1],
            v[v.len() * 99 / 100],
            v[v.len() / 2],
            v.iter().filter(|&&x| x == 0).count(),
            sq * v.len() as f64 / (tot as f64).powi(2)
        )
    };
    let mut rep = format!("{{\"n\": {n}, \"nq\": {}, \"insert_rows_s\": {insert_s:.3}, \"kmeans_s\": {kmeans_s:.3}, \"train_place_s\": {train_s:.3}, \"lists\": {ls}{retrain_rep}", gt.len());
    // WARM_L22_ACC_RULES: `;`-separated `fixed:NP`, `ratio:EPS:MIN:MAX`, `radius:EPS:SCALE:MIN:MAX`.
    // Each is searched in order after one discarded warm-up search; keys are `<idx>_<rule>`.
    let rules: Vec<(String, ProbeRule, usize, usize)> =
        crate::compatibility::var("WARM_L22_ACC_RULES")
            .map(|v| {
                v.split(';')
                    .map(|tok| {
                        let f: Vec<&str> = tok.split(':').collect();
                        let num = |i: usize| f[i].trim().parse::<f32>().unwrap();
                        match f[0] {
                            "fixed" => (ProbeRule::Fixed, 1, num(1) as usize),
                            "ratio" => (
                                ProbeRule::Ratio { eps: num(1) },
                                num(2) as usize,
                                num(3) as usize,
                            ),
                            "radius" => (
                                ProbeRule::Radius {
                                    eps: num(1),
                                    scale: num(2),
                                },
                                num(3) as usize,
                                num(4) as usize,
                            ),
                            other => panic!("rule {other}"),
                        }
                    })
                    .enumerate()
                    .map(|(i, (r, lo, hi))| (format!("{i}_{}", tokens(&v, i)), r, lo, hi))
                    .collect()
            })
            .unwrap_or_else(|_| {
                crate::compatibility::var("WARM_L22_ACC_NPROBES")
                    .map(|v| v.split(',').map(|x| x.trim().parse().unwrap()).collect())
                    .unwrap_or_else(|_| vec![350usize, 700])
                    .into_iter()
                    .map(|np| (format!("np{np}"), ProbeRule::Fixed, 1, np))
                    .collect()
            });
    if crate::compatibility::var("WARM_L22_ACC_RULES").is_ok() {
        t.set_nprobe(rules[0].3);
        let _ = t.search(&q, 10).unwrap();
    }
    let nsearch = (q.len() / dim) as f64;
    for (label, rule, lo, hi) in rules {
        t.set_nprobe(hi);
        t.set_probe_rule(rule, lo);
        let t0 = Instant::now();
        let got = t.search(&q, 10).unwrap();
        let s = t0.elapsed().as_secs_f64();
        let pf = t.last_search_profile();
        let digest = got
            .iter()
            .flatten()
            .fold(0xcbf2_9ce4_8422_2325u64, |h, &id| {
                (h ^ u64::from(id)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        let miss = {
            let tr = t.trained.as_ref().unwrap();
            let l = tr.lists_len();
            let mut per_q: Vec<usize> = Vec::new();
            let (mut ranks, mut sizes) = (Vec::new(), Vec::new());
            for (qi, (g, want)) in got.iter().zip(&gt).enumerate() {
                let miss: Vec<u32> = want.iter().copied().filter(|w| !g.contains(w)).collect();
                per_q.push(miss.len());
                if miss.is_empty() {
                    continue;
                }
                let qv = &q[qi * dim..(qi + 1) * dim];
                let d: Vec<f32> = (0..l)
                    .map(|c| sq(qv, &tr.centres[c * dim..(c + 1) * dim]))
                    .collect();
                for w in miss {
                    let e = tr.loc[w as usize];
                    let best = e
                        .iter()
                        .filter(|x| x.0 != u32::MAX)
                        .map(|x| x.0 as usize)
                        .min_by(|&a, &b| d[a].total_cmp(&d[b]))
                        .unwrap();
                    ranks.push(d.iter().filter(|&&x| x < d[best]).count());
                    sizes.push(tr.lists[best].slots.len());
                }
            }
            let total: usize = per_q.iter().sum();
            let mut sorted = per_q.clone();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            let top10: usize = sorted.iter().take(10).sum();
            ranks.sort_unstable();
            sizes.sort_unstable();
            let pick = |v: &Vec<usize>, f: usize| {
                v.get((v.len().saturating_sub(1)) * f / 100)
                    .copied()
                    .unwrap_or(0)
            };
            format!(
                "{{\"missed\": {total}, \"queries_missing\": {}, \"top10_queries\": {top10}, \"worst_q\": {}, \"rank_p50\": {}, \"rank_p90\": {}, \"rank_max\": {}, \"size_p50\": {}, \"size_p90\": {}}}",
                per_q.iter().filter(|&&x| x > 0).count(),
                sorted[0],
                pick(&ranks, 50), pick(&ranks, 90), pick(&ranks, 100),
                pick(&sizes, 50), pick(&sizes, 90)
            )
        };
        rep += &format!(", \"miss_{label}\": {miss}");
        rep += &format!(
            ", \"digest_{label}\": \"{digest:016x}\", \"recall_{label}\": {:.4}, \"search_{label}_s\": {s:.3}, \"profile_{label}\": \
             {{\"centres\": {:.3}, \"route\": {:.3}, \"scan\": {:.3}, \"scan_mean\": {:.3}, \"lut_mean\": {:.3}, \"probes_mean\": {:.1}, \"blocks\": {}, \"admitted\": {}, \
             \"reorder\": {:.3}, \"total\": {:.3}}}",
            recall(&got, &gt),
            pf.centres_s,
            pf.route_s,
            pf.scan_s,
            pf.scan_mean_s,
            pf.lut_mean_s,
            pf.probes as f64 / nsearch,
            pf.blocks,
            pf.admitted,
            pf.reorder_s,
            pf.total_s
        );
    }
    rep += "}";
    println!("{rep}");
    if let Ok(p) = crate::compatibility::var("WARM_L22_ACC_OUT") {
        std::fs::write(p, rep + "\n").unwrap();
    }
}
