//! Round 2 Lead A baseline: static binary fuse filter over full records (xorf crate), decoded by
//! enumerating all V^u candidate tuples. Measures bits per key, false-positive rate and enumeration time.
use std::time::Instant;
use xorf::{BinaryFuse16, BinaryFuse32, BinaryFuse8, Filter};

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

fn probe<F: Filter<u64>>(f: &F, salt: u64, count: u64) -> (u64, f64) {
    let t = Instant::now();
    let mut hits = 0u64;
    for i in 0..count {
        if f.contains(&mix(i ^ salt)) {
            hits += 1;
        }
    }
    (hits, t.elapsed().as_secs_f64())
}

fn report<F: Filter<u64>>(name: &str, f: &F, fp_bits: u32, n: usize, fpr_probes: u64) {
    let bits_per_key = f.len() as f64 * fp_bits as f64 / n as f64;
    let (fp, _) = probe(f, 0xDEAD_BEEF_0000_0000, fpr_probes);
    // enumeration cost: u=3 means 2^24 candidate tuples per query, u=2 means 2^16 (repeated to average)
    let (_, t3) = probe(f, 0x1234_5678_0000_0000, 1 << 24);
    let (_, t2) = probe(f, 0x2345_6789_0000_0000, 1 << 24);
    let ns_per_probe = t3 / (1u64 << 24) as f64 * 1e9;
    println!(
        "{{\"filter\":\"{}\",\"fp_bits\":{},\"n\":{},\"bits_per_key\":{:.3},\"fpr_probes\":{},\"false_positives\":{},\"fpr\":{:.3e},\"theory_fpr\":{:.3e},\"ns_per_probe\":{:.2},\"ms_per_query_u3\":{:.1},\"ms_per_query_u2\":{:.3},\"ms_per_query_u1\":{:.4}}}",
        name,
        fp_bits,
        n,
        bits_per_key,
        fpr_probes,
        fp,
        fp as f64 / fpr_probes as f64,
        2f64.powi(-(fp_bits as i32)),
        ns_per_probe,
        t3 * 1e3,
        t2 / 256.0 * 1e3,
        t2 / 65536.0 * 1e3
    );
}

fn main() {
    for n in [20_000usize, 200_000] {
        let keys: Vec<u64> = (0..n as u64).map(|i| mix(i ^ 0xA5A5_A5A5)).collect();
        let f8 = BinaryFuse8::try_from(&keys).expect("fuse8");
        let f16 = BinaryFuse16::try_from(&keys).expect("fuse16");
        let f32_ = BinaryFuse32::try_from(&keys).expect("fuse32");
        for k in &keys[..1000] {
            assert!(f8.contains(k) && f16.contains(k) && f32_.contains(k), "false negative");
        }
        report("BinaryFuse8", &f8, 8, n, 200_000_000);
        report("BinaryFuse16", &f16, 16, n, 1_000_000_000);
        report("BinaryFuse32", &f32_, 32, n, 2_000_000_000);
    }
}
