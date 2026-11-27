// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Refresh cost against store size (`docs/reference/concurrency.md`). Ignored by default: it
//! writes a journal of `WARM_READER_SCALE_MB` (default 1024) megabytes. Run it with
//! `cargo test --release --test reader_scale -- --ignored --nocapture`.

use std::path::Path;
use std::time::{Duration, Instant};
use writerslogic_warm::core::documents::DocumentInput;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{Refresh, WarmCore};

const DIM: u32 = 256;
const DOC_BYTES: usize = 32 * 1024;
const DOCS_PER_COMMIT: usize = 32;

fn words(state: &mut u64, bytes: usize) -> String {
    let mut out = String::with_capacity(bytes + 8);
    while out.len() < bytes {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        // A vocabulary of 2^20 pseudo-words keeps the text from compressing away.
        let mut w = *state & 0xF_FFFF;
        for _ in 0..4 {
            out.push((b'a' + (w % 26) as u8) as char);
            w /= 26;
        }
        out.push(' ');
    }
    out
}

fn doc(id: String, text: String) -> DocumentInput {
    serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
}

fn tree_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| match entry.metadata() {
                    Ok(meta) if meta.is_dir() => tree_bytes(&entry.path()),
                    Ok(meta) => meta.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// Builds a store of about `megabytes` of journal, opens a reader, and times refreshes that each
/// pick up one new document. Returns (store bytes, open time, median refresh time).
fn measure(megabytes: u64) -> (u64, Duration, Duration) {
    let dir = tempfile::tempdir().unwrap();
    let path = Some(dir.path().display().to_string());
    // No folding of the index into a checkpoint: that is a whole-store compaction by the writer
    // (after which a refresh reopens), not what this measures.
    let mut config = WarmConfig::unencrypted();
    config.document_checkpoint.index_memory_budget = 0;
    let config = Some(config);
    let writer = WarmCore::new(DIM, path.clone(), config.clone()).unwrap();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut n = 0usize;
    while tree_bytes(&dir.path().join("vectors_data.bin")) < megabytes << 20 {
        let docs = (0..DOCS_PER_COMMIT)
            .map(|_| {
                n += 1;
                doc(format!("d-{n}"), words(&mut state, DOC_BYTES))
            })
            .collect();
        writer.memorize_documents(docs).unwrap();
    }
    let size = tree_bytes(dir.path());
    let started = Instant::now();
    let reader = WarmCore::open_read_only(DIM, path, config).unwrap();
    let open = started.elapsed();
    let mut times = Vec::new();
    for i in 0..30 {
        writer
            .memorize_document(doc(format!("tail-{i}"), words(&mut state, DOC_BYTES)))
            .unwrap();
        let started = Instant::now();
        assert_eq!(reader.refresh_in_place().unwrap(), Refresh::Advanced);
        times.push(started.elapsed());
    }
    println!("  refreshes in order: {times:?}");
    times.sort();
    (size, open, times[times.len() / 2])
}

#[test]
#[ignore = "writes a journal of WARM_READER_SCALE_MB megabytes"]
fn refresh_time_is_flat_from_10_mb_to_a_large_journal() {
    let large: u64 = std::env::var("WARM_READER_SCALE_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024);
    let small = measure(10);
    let big = measure(large);
    for (label, (size, open, refresh)) in [("small", small), ("large", big)] {
        println!(
            "{label}: store {} MB, reader open {open:?}, median refresh {refresh:?}",
            size >> 20
        );
    }
    assert!(big.0 >= 10 * small.0, "the large store is not large");
    // One new 32 KB document costs the same to pick up beside either store; allow noise and
    // the larger store's colder caches, but nothing proportional to its size.
    assert!(
        big.2 < small.2 * 5 + Duration::from_millis(20),
        "refresh grew with the store: {:?} -> {:?}",
        small.2,
        big.2
    );
}
