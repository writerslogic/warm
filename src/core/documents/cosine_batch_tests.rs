// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::{xorshift64, Embedding};

#[test]
fn cosine4_equals_cosine_bit_for_bit() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || (xorshift64(&mut state) >> 40) as f32 / 8_388_608.0 - 1.0;
    for dim in [1usize, 3, 384] {
        let rows: Vec<Embedding> = (0..4)
            .map(|_| Embedding::from_f32((0..dim).map(|_| next()).collect()))
            .collect();
        let query: Vec<f64> = (0..dim).map(|_| f64::from(next())).collect();
        let batch = Embedding::cosine4([&rows[0], &rows[1], &rows[2], &rows[3]], &query);
        for (row, score) in rows.iter().zip(batch) {
            assert_eq!(row.cosine(&query).to_bits(), score.to_bits());
        }
    }
    let short = Embedding::from_f32(vec![0.5; 2]);
    let long = Embedding::from_f32(vec![0.25; 5]);
    let query = vec![1.0f64; 5];
    let batch = Embedding::cosine4([&long, &long, &short, &long], &query);
    assert_eq!(batch[2].to_bits(), short.cosine(&query).to_bits());
}
