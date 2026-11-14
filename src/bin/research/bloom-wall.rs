// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//
// Membership and binding checks for the vector-symbolic bundle.
//
// Part A (membership). Member-vs-non-member AUC (Mann-Whitney) over a sweep of
// stored-set sizes N, for: the old shipped setting (64 probes per item), the
// shipped memory sized from N (`with_expected_items`), the shipped default
// constructor (`new`), a plain Bloom filter at the same D bits with tuned k, a
// plain Bloom filter at the counting arm's D*32 bits, and the counting bundle
// (u32 counters, Poisson z-sum readout). Bits per arm are in the output.
//
// Part B (binding). Retrieve the value bound to a noisy key from a superposition
// of K key-value pairs, against two baselines at the same number of bits: a hash
// table keyed on the exact key, and a table of per-pair key sketches searched by
// Hamming distance. It reports accuracy only; it does not assume binding wins.
//
// Run: cargo run --release --features experimental --bin bloom-wall -- [--json PATH]

use writerslogic_warm::core::bloom_memory::HolographicBloomMemory;
use writerslogic_warm::core::counting_bloom_memory::CountingBloomMemory;
use writerslogic_warm::core::entangled::EntangledHVec;

const D: usize = 16384;
const OLD_K: usize = 64; // active indices per item at D = 16384
const SEEDS: u64 = 20;
const PROBE: usize = 200; // members / non-members sampled for AUC
const COUNT_BITS: usize = D * 32;
const LOADS: [usize; 14] = [
    100, 200, 400, 700, 1000, 1500, 2000, 3000, 4000, 6000, 8000, 11000, 14000, 20000,
];

fn mix(x: u64, k: u64) -> u64 {
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    h
}

/// AUC via Mann-Whitney: P(member score > non-member score), ties = 0.5.
fn auc(pos: &[f64], neg: &[f64]) -> f64 {
    let mut wins = 0.0f64;
    for &p in pos {
        for &n in neg {
            wins += if p > n {
                1.0
            } else if p == n {
                0.5
            } else {
                0.0
            };
        }
    }
    wins / (pos.len() as f64 * neg.len() as f64)
}

/// Plain Bloom filter: m bits, k hash positions per id, containment readout.
struct Bloom {
    m: usize,
    k: usize,
    bits: Vec<bool>,
}

impl Bloom {
    fn new(m: usize, n: usize) -> Self {
        let k = ((std::f64::consts::LN_2 * m as f64 / n as f64).round() as usize).max(1);
        Self {
            m,
            k,
            bits: vec![false; m],
        }
    }
    fn pos(&self, id: u64, seed: u64, j: usize) -> usize {
        (mix(id.wrapping_mul(0x100_0001).wrapping_add(j as u64), seed) % self.m as u64) as usize
    }
    fn insert(&mut self, id: u64, seed: u64) {
        for j in 0..self.k {
            let p = self.pos(id, seed, j);
            self.bits[p] = true;
        }
    }
    fn score(&self, id: u64, seed: u64) -> f64 {
        (0..self.k)
            .filter(|&j| self.bits[self.pos(id, seed, j)])
            .count() as f64
            / self.k as f64
    }
}

#[derive(Default, Clone)]
struct Row {
    n: usize,
    old_k64: f64,
    new_sized: f64,
    new_sized_probes: usize,
    new_default: f64,
    new_default_probes: usize,
    bloom_d: f64,
    bloom_d_k: usize,
    bloom_32d: f64,
    bloom_32d_k: usize,
    counting: f64,
}

fn eval(n: usize) -> Row {
    let mut row = Row {
        n,
        ..Default::default()
    };
    for seed in 0..SEEDS {
        let base = seed * 1_000_003;
        let items: Vec<EntangledHVec> = (0..n as u64)
            .map(|i| EntangledHVec::new_deterministic(D, base + i))
            .collect();
        let nonmembers: Vec<EntangledHVec> = (0..PROBE as u64)
            .map(|j| EntangledHVec::new_deterministic(D, base + 9_000_000 + j))
            .collect();
        let probe = PROBE.min(n);

        let run = |mem: &mut HolographicBloomMemory| -> f64 {
            mem.insert_batch(&items);
            let p: Vec<f64> = items[..probe].iter().map(|i| mem.contains(i)).collect();
            let q: Vec<f64> = nonmembers.iter().map(|i| mem.contains(i)).collect();
            auc(&p, &q)
        };
        let mut old = HolographicBloomMemory::with_expected_items(D, 177);
        assert_eq!(old.probes(), OLD_K);
        row.old_k64 += run(&mut old);
        let mut sized = HolographicBloomMemory::with_expected_items(D, n);
        row.new_sized_probes = sized.probes();
        row.new_sized += run(&mut sized);
        let mut dflt = HolographicBloomMemory::new(D);
        row.new_default_probes = dflt.probes();
        row.new_default += run(&mut dflt);

        let mut cm = CountingBloomMemory::new(D);
        cm.insert_batch(&items);
        let p: Vec<f64> = items[..probe]
            .iter()
            .map(|i| cm.membership_score(i))
            .collect();
        let q: Vec<f64> = nonmembers.iter().map(|i| cm.membership_score(i)).collect();
        row.counting += auc(&p, &q);

        for (m, acc, kslot) in [
            (D, &mut row.bloom_d, &mut row.bloom_d_k),
            (COUNT_BITS, &mut row.bloom_32d, &mut row.bloom_32d_k),
        ] {
            let mut b = Bloom::new(m, n);
            *kslot = b.k;
            for id in 0..n as u64 {
                b.insert(id, seed);
            }
            let p: Vec<f64> = (0..probe as u64).map(|id| b.score(id, seed)).collect();
            let q: Vec<f64> = (0..PROBE as u64)
                .map(|j| b.score(9_000_000 + j, seed))
                .collect();
            *acc += auc(&p, &q);
        }
    }
    let s = SEEDS as f64;
    row.old_k64 /= s;
    row.new_sized /= s;
    row.new_default /= s;
    row.bloom_d /= s;
    row.bloom_32d /= s;
    row.counting /= s;
    row
}

// ---- Part B: key-value binding -------------------------------------------

const BD: usize = 16384; // bipolar dimension
const VALUES: usize = 256; // value alphabet, 8 bits per id
const VALUE_BITS: usize = 8;
const COUNTER_BITS: usize = 8; // superposition counters clamped to +-127
const BIND_SEEDS: u64 = 5;
const BIND_PROBE: usize = 100;
const BIND_KS: [usize; 6] = [50, 100, 200, 400, 800, 1600];
const BIND_PS: [f64; 5] = [0.0, 0.1, 0.2, 0.3, 0.4];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0, 0x1234_5678_9ABC_DEF1)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn bipolar(rng: &mut Rng, d: usize) -> Vec<i8> {
    let mut v = Vec::with_capacity(d);
    while v.len() < d {
        let w = rng.next();
        for b in 0..64.min(d - v.len()) {
            v.push(if w >> b & 1 == 1 { 1 } else { -1 });
        }
    }
    v
}

#[derive(Default, Clone)]
struct BindRow {
    k: usize,
    p: f64,
    superposition: f64,
    exact_hash: f64,
    sketch_nn: f64,
    sketch_bits: usize,
}

fn bind_eval(k: usize, p: f64) -> BindRow {
    let budget = BD * COUNTER_BITS;
    let sketch_bits = (budget / k).saturating_sub(VALUE_BITS).min(BD);
    let mut row = BindRow {
        k,
        p,
        sketch_bits,
        ..Default::default()
    };
    for seed in 0..BIND_SEEDS {
        let mut rng = Rng(seed * 7919 + k as u64 * 31 + 1);
        let codes: Vec<Vec<i8>> = (0..VALUES).map(|_| bipolar(&mut rng, BD)).collect();
        let keys: Vec<Vec<i8>> = (0..k).map(|_| bipolar(&mut rng, BD)).collect();
        let vals: Vec<usize> = (0..k).map(|_| rng.below(VALUES)).collect();
        let mut acc = vec![0i32; BD];
        for (key, &v) in keys.iter().zip(&vals) {
            for d in 0..BD {
                acc[d] += (key[d] * codes[v][d]) as i32;
            }
        }
        let acc: Vec<i16> = acc.iter().map(|&a| a.clamp(-127, 127) as i16).collect();

        let (mut ok_sup, mut ok_hash, mut ok_sk) = (0usize, 0usize, 0usize);
        let probe = BIND_PROBE.min(k);
        for q in 0..probe {
            let noisy: Vec<i8> = keys[q]
                .iter()
                .map(|&b| if rng.unit() < p { -b } else { b })
                .collect();
            // superposition: unbind then clean up against the value codebook
            let u: Vec<i16> = acc
                .iter()
                .zip(&noisy)
                .map(|(&a, &b)| a * b as i16)
                .collect();
            let best = (0..VALUES)
                .max_by_key(|&v| {
                    u.iter()
                        .zip(&codes[v])
                        .map(|(&x, &c)| x as i32 * c as i32)
                        .sum::<i32>()
                })
                .unwrap();
            ok_sup += (best == vals[q]) as usize;
            // exact-key hash table: a noisy key hashes elsewhere, answer is a guess
            let guess = if noisy == keys[q] {
                vals[q]
            } else {
                rng.below(VALUES)
            };
            ok_hash += (guess == vals[q]) as usize;
            // key sketches: nearest stored sketch by Hamming distance
            if sketch_bits > 0 {
                let nearest = (0..k)
                    .min_by_key(|&j| (0..sketch_bits).filter(|&d| keys[j][d] != noisy[d]).count())
                    .unwrap();
                ok_sk += (vals[nearest] == vals[q]) as usize;
            } else {
                ok_sk += (rng.below(VALUES) == vals[q]) as usize;
            }
        }
        let denom = (probe as u64 * BIND_SEEDS) as f64;
        row.superposition += ok_sup as f64 / denom;
        row.exact_hash += ok_hash as f64 / denom;
        row.sketch_nn += ok_sk as f64 / denom;
    }
    row
}

fn main() {
    let json_path = std::env::args().skip_while(|a| a != "--json").nth(1);

    println!("bloom-wall | D={D} seeds={SEEDS} | membership AUC (chance 0.50), 200 members vs 200 non-members");
    println!("bits: bundle arms {D}, counting {COUNT_BITS}, bloom_32d {COUNT_BITS}, bloom_d {D}");
    println!(
        "{:>6} {:>8} {:>9} {:>9} {:>8} {:>8} {:>8}   probes(sized/default) bloom_d_k",
        "n", "old_k64", "new_size", "new_dflt", "bloom_D", "bloom32D", "count"
    );
    let mut rows = Vec::new();
    for n in LOADS {
        let r = eval(n);
        println!(
            "{:>6} {:>8.4} {:>9.4} {:>9.4} {:>8.4} {:>8.4} {:>8.4}   {}/{} {}",
            r.n,
            r.old_k64,
            r.new_sized,
            r.new_default,
            r.bloom_d,
            r.bloom_32d,
            r.counting,
            r.new_sized_probes,
            r.new_default_probes,
            r.bloom_d_k
        );
        rows.push(r);
    }
    let wall = |f: &dyn Fn(&Row) -> f64, thr: f64| -> String {
        rows.iter()
            .find(|r| f(r) < thr)
            .map_or(format!(">{}", LOADS[LOADS.len() - 1]), |r| r.n.to_string())
    };
    println!("\nfirst n with AUC below 0.99 / 0.95:");
    type Arm<'a> = (&'a str, &'a dyn Fn(&Row) -> f64);
    let arms: [Arm; 6] = [
        ("old_k64", &|r| r.old_k64),
        ("new_sized", &|r| r.new_sized),
        ("new_default", &|r| r.new_default),
        ("bloom_d", &|r| r.bloom_d),
        ("bloom_32d", &|r| r.bloom_32d),
        ("counting", &|r| r.counting),
    ];
    for (name, f) in arms {
        println!("  {name:<12} {:>7} / {:>7}", wall(f, 0.99), wall(f, 0.95));
    }

    println!(
        "\nbinding | D={BD} values={VALUES} counters={COUNTER_BITS}b budget={} bits seeds={BIND_SEEDS} probe={BIND_PROBE}",
        BD * COUNTER_BITS
    );
    println!(
        "{:>5} {:>5} {:>12} {:>10} {:>10} {:>11}   (accuracy of the recovered value; chance {:.4})",
        "K",
        "p",
        "superpos",
        "exact_hash",
        "sketch_nn",
        "sketch_bits",
        1.0 / VALUES as f64
    );
    let mut brows = Vec::new();
    for k in BIND_KS {
        for p in BIND_PS {
            let r = bind_eval(k, p);
            println!(
                "{:>5} {:>5.2} {:>12.4} {:>10.4} {:>10.4} {:>11}",
                r.k, r.p, r.superposition, r.exact_hash, r.sketch_nn, r.sketch_bits
            );
            brows.push(r);
        }
    }

    if let Some(path) = json_path {
        let mut s = String::new();
        s.push_str(&format!(
            "{{\n \"D\": {D},\n \"seeds\": {SEEDS},\n \"bits\": {{\"bundle\": {D}, \"counting\": {COUNT_BITS}, \"bloom_d\": {D}, \"bloom_32d\": {COUNT_BITS}}},\n \"old_k64_probes\": {OLD_K},\n \"auc_mean\": {{\n"
        ));
        for (i, r) in rows.iter().enumerate() {
            s.push_str(&format!(
                "  \"{}\": {{\"old_k64\": {}, \"new_sized\": {}, \"new_sized_probes\": {}, \"new_default\": {}, \"new_default_probes\": {}, \"bloom_d\": {}, \"bloom_d_k\": {}, \"bloom_32d\": {}, \"bloom_32d_k\": {}, \"counting\": {}}}{}\n",
                r.n, r.old_k64, r.new_sized, r.new_sized_probes, r.new_default, r.new_default_probes,
                r.bloom_d, r.bloom_d_k, r.bloom_32d, r.bloom_32d_k, r.counting,
                if i + 1 < rows.len() { "," } else { "" }
            ));
        }
        s.push_str(&format!(
            " }},\n \"binding\": {{\n  \"dim\": {BD}, \"values\": {VALUES}, \"budget_bits\": {}, \"seeds\": {BIND_SEEDS}, \"probe\": {BIND_PROBE},\n  \"accuracy\": [\n",
            BD * COUNTER_BITS
        ));
        for (i, r) in brows.iter().enumerate() {
            s.push_str(&format!(
                "   {{\"k\": {}, \"p\": {}, \"superposition\": {}, \"exact_hash\": {}, \"sketch_nn\": {}, \"sketch_bits\": {}}}{}\n",
                r.k, r.p, r.superposition, r.exact_hash, r.sketch_nn, r.sketch_bits,
                if i + 1 < brows.len() { "," } else { "" }
            ));
        }
        s.push_str("  ]\n }\n}\n");
        std::fs::write(&path, s).expect("write json");
        println!("wrote {path}");
    }
}
